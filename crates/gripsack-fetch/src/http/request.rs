use super::Client;
use super::body::{BodyReadFailure, MetadataDecodeFailure, ResponseReader, TransferBudget};
use super::failure::{
    AuthenticationDisposition, HttpFailure, HttpFailureKind, safe_url, transport_kind,
};
use super::retry::{RequestBudget, RetryDecision, RetryStopReason};
use crate::FetchError;
use std::io::{self, Read};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Secret-bearing registry authorization is deliberately not Debug/Serialize.
#[derive(Clone, Copy)]
pub(crate) enum RequestKind<'a> {
    GithubMetadata,
    Metadata,
    Text,
    Artifact { api_url: Option<&'a str> },
    RegistryArtifact { authorization: &'a str },
}
impl RequestKind<'_> {
    fn label(self) -> &'static str {
        match self {
            Self::GithubMetadata => "github-metadata",
            Self::Metadata => "metadata",
            Self::Text => "text",
            Self::Artifact { .. } | Self::RegistryArtifact { .. } => "artifact",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Cooldown {
    pub until: Option<Instant>,
    pub kind: HttpFailureKind,
}

enum ServerDelay {
    None,
    Minimum(Duration),
    Unknown,
}

fn content_length(response: &ureq::Response) -> Option<u64> {
    if response.header("transfer-encoding").is_some()
        || response.header("content-encoding").is_some()
    {
        return None;
    }
    response
        .header("content-length")
        .and_then(|value| value.parse().ok())
}
fn delay_from_headers(response: &ureq::Response, primary: bool, limited: bool) -> ServerDelay {
    let now = SystemTime::now();
    let mut wait = None;
    if let Some(value) = response.header("retry-after") {
        wait = match value.trim().parse::<u64>() {
            Ok(seconds) => Some(Duration::from_secs(seconds)),
            Err(_) => match httpdate::parse_http_date(value) {
                Ok(time) => Some(time.duration_since(now).unwrap_or_default()),
                Err(_) => return ServerDelay::Unknown,
            },
        };
    }
    if primary {
        let Some(reset) = response
            .header("x-ratelimit-reset")
            .and_then(|text| text.parse::<u64>().ok())
            .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
        else {
            return ServerDelay::Unknown;
        };
        let reset_wait = reset.duration_since(now).unwrap_or_default();
        wait = Some(wait.map_or(reset_wait, |previous: Duration| previous.max(reset_wait)));
    }
    match wait {
        Some(wait) => ServerDelay::Minimum(wait),
        None if limited => ServerDelay::Minimum(Duration::from_secs(60)),
        None => ServerDelay::None,
    }
}

impl Client {
    pub(crate) fn json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        kind: RequestKind<'_>,
    ) -> Result<T, FetchError> {
        self.consume(url, kind, 8 * 1024 * 1024, |reader| {
            serde_json::from_reader(reader).map_err(|error| {
                if error.is_io() {
                    io::Error::from(error)
                } else {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        MetadataDecodeFailure {
                            line: error.line(),
                            column: error.column(),
                        },
                    )
                }
            })
        })
    }

    pub(crate) fn text(&self, url: &str, kind: RequestKind<'_>) -> Result<String, FetchError> {
        self.consume(url, kind, 8 * 1024 * 1024, |reader| {
            let mut output = String::new();
            reader.read_to_string(&mut output)?;
            Ok(output)
        })
    }

    pub(crate) fn download(
        &self,
        url: &str,
        kind: RequestKind<'_>,
        limit: u64,
    ) -> Result<crate::spool::Download, FetchError> {
        self.consume(url, kind, limit, |reader| {
            crate::spool::download(reader, limit)
        })
    }

    pub(crate) fn payload_hash(
        &self,
        url: &str,
        api_url: Option<&str>,
        limit: u64,
    ) -> Result<crate::DownloadHash, FetchError> {
        self.consume(url, RequestKind::Artifact { api_url }, limit, |reader| {
            crate::spool::copy_hashed(reader, io::sink(), limit)
        })
    }

    pub(crate) fn consume<T>(
        &self,
        url: &str,
        kind: RequestKind<'_>,
        limit: u64,
        mut consume: impl FnMut(&mut dyn Read) -> io::Result<T>,
    ) -> Result<T, FetchError> {
        let api_url = match kind {
            RequestKind::Artifact { api_url } => api_url,
            _ => None,
        };
        let selected = api_url
            .filter(|api| self.policy.header(api).is_some())
            .unwrap_or(url);
        let safe = safe_url(selected);
        let host = super::host_port(selected).map(|(host, _)| host);
        let _request_span =
            tracing::info_span!("http", url = %safe, purpose = kind.label()).entered();
        let mut budget = RequestBudget::new(Instant::now());
        let mut transfer = TransferBudget::new(limit);
        let authentication = || match kind {
            RequestKind::RegistryArtifact { .. } => AuthenticationDisposition::OtherBound,
            _ => self.policy.authentication(selected),
        };
        let failure = |kind, budget: &RequestBudget, stop| {
            let mut error = HttpFailure::new(selected, kind, budget, stop, authentication());
            error.api_host = api_url.and_then(super::host_port).map(|(host, _)| host);
            error
        };
        if let Some(host) = &host {
            let mut cooldowns = self.cooldowns.lock().expect("HTTP cooldown lock");
            if let Some(cooldown) = cooldowns.get(host).copied() {
                if cooldown.until.is_none_or(|until| until > Instant::now()) {
                    let mut error =
                        failure(cooldown.kind, &budget, RetryStopReason::ServerCooldown);
                    error.server_wait = cooldown
                        .until
                        .map(|until| until.saturating_duration_since(Instant::now()));
                    if matches!(kind, RequestKind::GithubMetadata) {
                        error.github_context(Some(selected));
                    }
                    return Err(error.into());
                }
                cooldowns.remove(host);
            }
        }
        loop {
            if !crate::throttle::acquire_url_until(selected, budget.deadline) {
                return Err(
                    failure(HttpFailureKind::Timeout, &budget, RetryStopReason::Deadline).into(),
                );
            }
            let mut request = self.request(selected).set("User-Agent", "gripsack");
            let remaining = budget.begin(Instant::now()).map_err(|stop| {
                FetchError::from(failure(HttpFailureKind::Timeout, &budget, stop))
            })?;
            request = request.timeout(remaining);
            let authorization = match kind {
                RequestKind::RegistryArtifact { authorization } => Some(authorization),
                _ => self.policy.header(selected),
            };
            if let Some(header) = authorization {
                request = request.set("Authorization", header);
            }
            if api_url.is_some_and(|api| api == selected) {
                request = request.set("Accept", "application/octet-stream");
            }
            tracing::info!(attempt = budget.attempts, "HTTP policy attempt");
            let (failure_kind, effective, server_delay) = match request.call() {
                Ok(response) => {
                    let effective = safe_url(response.get_url());
                    let expected_remaining = content_length(&response);
                    let html = response.header("content-type").is_some_and(|value| {
                        value
                            .split(';')
                            .next()
                            .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/html"))
                    });
                    if html {
                        (
                            HttpFailureKind::LoginPage,
                            Some(effective),
                            ServerDelay::None,
                        )
                    } else {
                        let mut reader = ResponseReader {
                            inner: response.into_reader(),
                            budget: &mut transfer,
                            deadline: budget.deadline,
                            expected_remaining,
                        };
                        match consume(&mut reader) {
                            Ok(value) => {
                                tracing::info!(
                                    attempts = budget.attempts,
                                    elapsed_ms = budget.started.elapsed().as_millis() as u64,
                                    "HTTP operation completed"
                                );
                                return Ok(value);
                            }
                            Err(error) => {
                                let classified = error
                                    .get_ref()
                                    .and_then(|cause| cause.downcast_ref::<BodyReadFailure>())
                                    .map(|failure| failure.kind)
                                    .or_else(|| {
                                        error
                                            .get_ref()
                                            .filter(|cause| cause.is::<MetadataDecodeFailure>())
                                            .map(|_| HttpFailureKind::InvalidMetadata)
                                    });
                                match classified {
                                    Some(kind) => (kind, Some(effective), ServerDelay::None),
                                    None => {
                                        tracing::warn!(
                                            attempts = budget.attempts,
                                            stop = "permanent resource or local I/O failure",
                                            "HTTP consumption failed without replay"
                                        );
                                        return Err(error.into());
                                    }
                                }
                            }
                        }
                    }
                }
                Err(ureq::Error::Status(status, response)) => {
                    let effective = safe_url(response.get_url());
                    let headers_at = Instant::now();
                    let expected_remaining = content_length(&response);
                    let primary = response.header("x-ratelimit-remaining") == Some("0");
                    let has_delay = response.header("retry-after").is_some();
                    let delay = delay_from_headers(&response, primary, status == 429);
                    let mut error_body = Vec::new();
                    let mut reader = ResponseReader {
                        inner: response.into_reader(),
                        budget: &mut transfer,
                        deadline: budget.deadline,
                        expected_remaining,
                    };
                    // Only classification is retained; never echo response bodies or header values.
                    let _ = reader.by_ref().take(8192).read_to_end(&mut error_body);
                    let secondary = status == 403
                        && String::from_utf8_lossy(&error_body)
                            .to_ascii_lowercase()
                            .contains("secondary rate limit");
                    let limited =
                        status == 429 || (status == 403 && (primary || has_delay || secondary));
                    let delay = if secondary && matches!(delay, ServerDelay::None) {
                        ServerDelay::Minimum(Duration::from_secs(60))
                    } else {
                        delay
                    };
                    let delay = match delay {
                        ServerDelay::Minimum(wait) => {
                            ServerDelay::Minimum(wait.saturating_sub(headers_at.elapsed()))
                        }
                        other => other,
                    };
                    (
                        if limited {
                            HttpFailureKind::RateLimited(status)
                        } else {
                            HttpFailureKind::Status(status)
                        },
                        Some(effective),
                        delay,
                    )
                }
                Err(ureq::Error::Transport(error)) => (
                    transport_kind(&error),
                    error.url().map(|url| safe_url(url.as_str())),
                    ServerDelay::None,
                ),
            };
            if matches!(failure_kind, HttpFailureKind::RateLimited(_)) {
                // Apply evidence to its actual response host, not an unrelated forge.
                let cooled_host = effective
                    .as_deref()
                    .and_then(super::host_port)
                    .map(|(host, _)| host)
                    .or_else(|| host.clone());
                if let Some(host) = cooled_host {
                    let until = match server_delay {
                        ServerDelay::Minimum(wait) => Instant::now().checked_add(wait),
                        _ => None,
                    };
                    self.cooldowns.lock().expect("HTTP cooldown lock").insert(
                        host,
                        Cooldown {
                            until,
                            kind: failure_kind,
                        },
                    );
                }
            }
            let server_wait = match server_delay {
                ServerDelay::Minimum(wait) => Some(wait),
                _ => None,
            };
            let decision = if matches!(server_delay, ServerDelay::Unknown) {
                RetryDecision::Stop(RetryStopReason::UnknownServerDelay)
            } else {
                budget.decide(Instant::now(), failure_kind, server_wait)
            };
            match decision {
                RetryDecision::RetryAfter(wait) if transfer.remaining != 0 => {
                    tracing::warn!(attempt = budget.attempts, failure = %failure_kind, wait_ms = wait.as_millis() as u64, "retrying HTTP policy attempt");
                    budget.waited += wait;
                    std::thread::sleep(wait);
                }
                RetryDecision::RetryAfter(_) => {
                    return Err(FetchError::PayloadTooLarge {
                        what: "HTTP transfer across attempts".into(),
                        limit,
                    });
                }
                RetryDecision::Stop(stop) => {
                    let mut error = failure(failure_kind, &budget, stop);
                    error.effective_url = effective.filter(|effective| *effective != safe);
                    error.server_wait = server_wait;
                    if matches!(kind, RequestKind::GithubMetadata) {
                        error.github_context(Some(selected));
                    }
                    tracing::warn!(attempts = budget.attempts, failure = %failure_kind, stop = %stop, "HTTP operation failed");
                    return Err(error.into());
                }
            }
        }
    }
}
