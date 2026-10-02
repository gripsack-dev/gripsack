use super::body::{BodyReadFailure, MetadataDecodeFailure, ResponseReader, TransferBudget};
use super::failure::{
    AuthenticationDisposition, HttpFailure, HttpFailureKind, safe_url, transport_kind,
};
use super::retry::{RequestBudget, RetryDecision, RetryStopReason};
use super::{Client, CredentialRoute};
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
        consume: impl FnMut(&mut dyn Read) -> io::Result<T>,
    ) -> Result<T, FetchError> {
        self.consume_at(url, kind, limit, Instant::now(), consume)
    }

    fn consume_at<T>(
        &self,
        url: &str,
        kind: RequestKind<'_>,
        limit: u64,
        started: Instant,
        mut consume: impl FnMut(&mut dyn Read) -> io::Result<T>,
    ) -> Result<T, FetchError> {
        let api_url = match kind {
            RequestKind::Artifact { api_url } => api_url,
            _ => None,
        };
        // Select a bound API URL even when it is cleartext: dropping it
        // would silently fetch the browser URL without the intended auth.
        // Parse the selected route once; retry attempts reuse its header.
        let route_for = |value| {
            if matches!(kind, RequestKind::RegistryArtifact { .. }) {
                CredentialRoute::Unbound
            } else {
                self.policy.route(value)
            }
        };
        let (selected, credential) = match api_url {
            Some(api) => match route_for(api) {
                CredentialRoute::Unbound => (url, route_for(url)),
                bound => (api, bound),
            },
            None => (url, route_for(url)),
        };
        let safe = safe_url(selected);
        let host = super::host_port(selected).map(|(host, _)| host);
        let _request_span =
            tracing::info_span!("http", url = %safe, purpose = kind.label()).entered();
        let mut budget = RequestBudget::new(started)?;
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
        let admission_failure = |budget: &RequestBudget, stop: RetryStopReason| -> FetchError {
            if stop == RetryStopReason::ProtocolOrder {
                io::Error::new(io::ErrorKind::InvalidData, stop.to_string()).into()
            } else {
                failure(HttpFailureKind::Timeout, budget, stop).into()
            }
        };
        // Refuse the entire operation rather than silently dropping a bound
        // credential and falling back to a cleartext API/browser URL.
        let registry_auth = matches!(kind, RequestKind::RegistryArtifact { .. });
        let insecure_registry =
            registry_auth && !url::Url::parse(selected).is_ok_and(|url| url.scheme() == "https");
        if matches!(credential, CredentialRoute::Insecure) || insecure_registry {
            budget.stop(gripsack_policy::retry_budget::RetryRefusal::NonRetryable);
            return Err(failure(
                HttpFailureKind::InsecureCredential,
                &budget,
                RetryStopReason::NonRetryable,
            )
            .into());
        }
        if let Some(host) = &host {
            let mut cooldowns = self
                .cooldowns
                .try_lock_until(budget.deadline())
                .ok_or_else(|| {
                    let stop = budget.stop(gripsack_policy::retry_budget::RetryRefusal::Deadline);
                    admission_failure(&budget, stop)
                })?;
            if let Some(cooldown) = cooldowns.get(host).copied() {
                if cooldown.until.is_none_or(|until| until > Instant::now()) {
                    budget.stop(gripsack_policy::retry_budget::RetryRefusal::ServerCooldown);
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
            if crate::throttle::acquire_url_until(selected, budget.deadline())
                == crate::throttle::ThrottleAdmission::DeadlineExpired
            {
                budget.stop(gripsack_policy::retry_budget::RetryRefusal::Deadline);
                return Err(
                    failure(HttpFailureKind::Timeout, &budget, RetryStopReason::Deadline).into(),
                );
            }
            let mut request = self.request(selected).set("User-Agent", "gripsack");
            let remaining = budget
                .begin(Instant::now())
                .map_err(|stop| admission_failure(&budget, stop))?;
            request = request.timeout(remaining);
            let authorization = match kind {
                RequestKind::RegistryArtifact { authorization } => Some(authorization),
                _ => match credential {
                    CredentialRoute::Https(header) => Some(header),
                    CredentialRoute::Unbound | CredentialRoute::Insecure => None,
                },
            };
            if let Some(header) = authorization {
                request = request.set("Authorization", header);
            }
            if api_url.is_some_and(|api| api == selected) {
                request = request.set("Accept", "application/octet-stream");
            }
            tracing::info!(attempt = budget.attempts(), "HTTP policy attempt");
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
                            deadline: budget.deadline(),
                            expected_remaining,
                        };
                        match consume(&mut reader) {
                            Ok(value) => {
                                budget
                                    .complete(Instant::now())
                                    .map_err(|stop| admission_failure(&budget, stop))?;
                                tracing::info!(
                                    attempts = budget.attempts(),
                                    elapsed_ms = budget.started().elapsed().as_millis() as u64,
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
                                        budget.stop(gripsack_policy::retry_budget::RetryRefusal::NonRetryable);
                                        tracing::warn!(
                                            attempts = budget.attempts(),
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
                        deadline: budget.deadline(),
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
                    let mut cooldowns = self
                        .cooldowns
                        .try_lock_until(budget.deadline())
                        .ok_or_else(|| {
                            let stop =
                                budget.stop(gripsack_policy::retry_budget::RetryRefusal::Deadline);
                            admission_failure(&budget, stop)
                        })?;
                    cooldowns.insert(
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
                RetryDecision::Stop(
                    budget.stop(gripsack_policy::retry_budget::RetryRefusal::UnknownServerDelay),
                )
            } else {
                budget.decide(Instant::now(), failure_kind, server_wait)
            };
            match decision {
                RetryDecision::RetryAfter(wait) if transfer.remaining != 0 => {
                    tracing::warn!(attempt = budget.attempts(), failure = %failure_kind, wait_ms = wait.as_millis() as u64, "retrying HTTP policy attempt");
                    std::thread::sleep(wait);
                }
                RetryDecision::RetryAfter(_) => {
                    budget.stop(gripsack_policy::retry_budget::RetryRefusal::NonRetryable);
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
                    tracing::warn!(attempts = budget.attempts(), failure = %failure_kind, stop = %stop, "HTTP operation failed");
                    return Err(error.into());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::Write;
    use std::net::TcpListener;

    fn loopback_client() -> Client {
        let mut client = Client::from_env(&Default::default());
        client.proxy = None;
        client.policy.no_proxy = "*".into();
        client.policy.github = None;
        client.policy.enterprise = None;
        client.policy.enterprise_host = None;
        client
    }

    #[test]
    fn a_successful_body_consumer_cannot_complete_after_the_operation_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/payload", listener.local_addr().unwrap());
        let client = loopback_client();
        // Load platform CA locations before the short, already-spent operation.
        // This builds the real agent but does not open a network connection.
        let _ = client.request(&url);
        std::thread::scope(|scope| {
            let server = scope.spawn(move || {
                let end = Instant::now() + Duration::from_secs(2);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == io::ErrorKind::WouldBlock
                                && Instant::now() < end =>
                        {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("loopback HTTP admission failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0u8; 4096];
                let mut used = 0;
                while !request[..used].ends_with(b"\r\n\r\n") {
                    assert!(used < request.len());
                    let count = stream.read(&mut request[used..]).unwrap();
                    assert!(count > 0);
                    used += count;
                }
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody",
                    )
                    .unwrap();
            });
            let end = Instant::now() + Duration::from_millis(500);
            let started = end - super::super::retry::OPERATION_TIMEOUT;
            let consumed = Cell::new(false);
            let result = client.consume_at(&url, RequestKind::Text, 8, started, |reader| {
                let mut body = String::new();
                reader.read_to_string(&mut body)?;
                consumed.set(true);
                std::thread::sleep(
                    end.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
                );
                Ok(body)
            });
            server.join().unwrap();
            assert!(
                consumed.get(),
                "late-completion fixture did not exercise the real body consumer"
            );
            assert!(
                matches!(&result, Err(FetchError::Http(failure)) if failure.kind() == HttpFailureKind::Timeout),
                "late_http_consumer_was_reported_successfully: {result:?}",
            );
        });
    }

    #[test]
    fn cooldown_contention_cannot_extend_the_original_operation_deadline() {
        let client = loopback_client();
        std::thread::scope(|scope| {
            let held = client.cooldowns.lock();
            let (send, receive) = std::sync::mpsc::channel();
            let shared = &client;
            let worker = scope.spawn(move || {
                let end = Instant::now() + Duration::from_millis(30);
                let started = end - super::super::retry::OPERATION_TIMEOUT;
                let result = shared.consume_at(
                    "http://127.0.0.1:9/never-requested",
                    RequestKind::Text,
                    8,
                    started,
                    |_| Ok(()),
                );
                send.send(result).unwrap();
            });
            let result = receive.recv_timeout(Duration::from_secs(2));
            drop(held);
            worker.join().unwrap();
            let result = result.expect("deadline_blocked_by_http_cooldown_lock");
            assert!(
                matches!(&result, Err(FetchError::Http(failure)) if failure.kind() == HttpFailureKind::Timeout && failure.attempts() == 0),
                "expired_http_cooldown_admitted_work: {result:?}",
            );
        });
    }
}
