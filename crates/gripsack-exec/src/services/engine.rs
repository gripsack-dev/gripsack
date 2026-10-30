//! Bounded Docker Engine HTTP over one explicitly selected local Unix socket.
//! No docker subprocess, context discovery, TCP fallback or human-output parsing.
use super::{ServiceError, failure};
use reqwest::{Method, Url, blocking::Client};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const API_VERSION: &str = "v1.47";
const MAX_METADATA_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct Engine {
    client: Client,
    deadline: Instant,
    pub socket: PathBuf,
    pub identity: String,
}
impl Engine {
    pub fn connect(socket: Option<&Path>, deadline: Instant) -> Result<Self, ServiceError> {
        let socket = match socket {
            Some(path) => path.to_path_buf(),
            None => match std::env::var("DOCKER_HOST") {
                Ok(host) => PathBuf::from(host.strip_prefix("unix://").ok_or_else(|| failure("services require a local unix:// Docker endpoint; TCP and SSH endpoints are not supported"))?),
                Err(std::env::VarError::NotPresent) => PathBuf::from("/var/run/docker.sock"),
                Err(_) => return Err(failure("DOCKER_HOST is not valid UTF-8")),
            },
        };
        if !socket.is_absolute() {
            return Err(failure("Docker socket must be an absolute path"));
        }
        let client = crate::http_client::builder()
            .unix_socket(socket.clone())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|_| failure("could not initialize the Docker Unix-socket client"))?;
        let mut engine = Self {
            client,
            deadline,
            socket,
            identity: String::new(),
        };
        let info = engine
            .get(&["info"])?
            .ok_or_else(|| failure("Docker Engine API v1.47 is unavailable"))?;
        if info.get("OSType").and_then(Value::as_str) != Some("linux") {
            return Err(failure("services require a Linux Docker Engine"));
        }
        engine.identity = info
            .get("ID")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| failure("Docker returned no engine identity"))?
            .to_owned();
        Ok(engine)
    }

    fn request(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, &str)],
        body: Option<&Value>,
        limit: u64,
    ) -> Result<(u16, Vec<u8>), ServiceError> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| failure("service operation deadline expired"))?;
        let mut url = Url::parse("http://localhost/").expect("constant URL");
        url.path_segments_mut()
            .expect("hierarchical URL")
            .push(API_VERSION)
            .extend(segments.iter().copied());
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        let mut request = self.client.request(method, url).timeout(remaining);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().map_err(|_| failure("Docker request failed or timed out; inspect service status before retrying a mutation"))?;
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(failure("Docker response exceeds its byte bound"));
        }
        let mut bytes = Vec::new();
        response
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                failure("Docker response was interrupted; mutation outcome may be unknown")
            })?;
        if bytes.len() as u64 > limit {
            return Err(failure("Docker response exceeds its byte bound"));
        }
        if Instant::now() >= self.deadline {
            return Err(failure(
                "service operation deadline expired while reading Docker response",
            ));
        }
        Ok((status, bytes))
    }

    pub fn get(&self, segments: &[&str]) -> Result<Option<Value>, ServiceError> {
        let (status, bytes) = self.request(Method::GET, segments, &[], None, MAX_METADATA_BYTES)?;
        if status == 404 {
            return Ok(None);
        }
        if status != 200 {
            return Err(failure(format!(
                "Docker inspection refused (HTTP {status}); no ownership inferred"
            )));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| failure("Docker returned malformed inspection JSON"))
    }

    pub fn mutate(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, &str)],
        body: Option<&Value>,
        accepted: &[u16],
    ) -> Result<Value, ServiceError> {
        let (status, bytes) = self.request(method, segments, query, body, MAX_METADATA_BYTES)?;
        if !accepted.contains(&status) {
            // Engine error bodies may echo environment values. Do not persist or
            // render them; the operation and structured status are sufficient.
            return Err(failure(format!(
                "Docker mutation refused (HTTP {status}); inspect current service state before retrying"
            )));
        }
        if bytes.is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_slice(&bytes).map_err(|_| {
                failure("Docker returned malformed mutation JSON; outcome may be unknown")
            })
        }
    }

    pub fn ensure_image(&self, reference: &str) -> Result<(), ServiceError> {
        if self.get(&["images", reference, "json"])?.is_none() {
            let (status, bytes) = self.request(
                Method::POST,
                &["images", "create"],
                &[("fromImage", reference), ("platform", "linux/amd64")],
                None,
                MAX_METADATA_BYTES,
            )?;
            if status != 200 {
                return Err(failure(format!(
                    "Docker image acquisition failed (HTTP {status})"
                )));
            }
            for record in serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>() {
                let record = record
                    .map_err(|_| failure("Docker image pull returned malformed progress JSON"))?;
                if record.get("error").is_some() || record.get("errorDetail").is_some() {
                    return Err(failure(
                        "Docker could not acquire the pinned image; no container was created",
                    ));
                }
            }
        }
        let image = self
            .get(&["images", reference, "json"])?
            .ok_or_else(|| failure("pinned image remains absent after acquisition"))?;
        if image.get("Os").and_then(Value::as_str) != Some("linux")
            || image.get("Architecture").and_then(Value::as_str) != Some("amd64")
        {
            return Err(failure("service image must provide linux/amd64"));
        }
        Ok(())
    }

    pub fn logs(&self, container: &str, lines: u16) -> Result<Vec<u8>, ServiceError> {
        let lines = lines.to_string();
        let (status, bytes) = self.request(
            Method::GET,
            &["containers", container, "logs"],
            &[
                ("stdout", "true"),
                ("stderr", "true"),
                ("tail", &lines),
                ("follow", "false"),
            ],
            None,
            MAX_LOG_BYTES,
        )?;
        if status != 200 {
            return Err(failure(format!(
                "Docker log request refused (HTTP {status})"
            )));
        }
        let mut output = Vec::with_capacity(bytes.len());
        let mut cursor = 0;
        while cursor < bytes.len() {
            let header = bytes
                .get(cursor..cursor + 8)
                .ok_or_else(|| failure("truncated Docker log frame"))?;
            if !matches!(header[0], 1 | 2) || header[1..4] != [0, 0, 0] {
                return Err(failure("invalid Docker log frame"));
            }
            let length = u32::from_be_bytes(header[4..8].try_into().expect("four bytes")) as usize;
            cursor += 8;
            let end = cursor
                .checked_add(length)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| failure("truncated Docker log payload"))?;
            output.extend_from_slice(&bytes[cursor..end]);
            cursor = end;
        }
        Ok(output)
    }
}
