use super::retry::{RequestBudget, RetryStopReason};
use std::error::Error;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpFailureKind {
    Status(u16),
    RateLimited(u16),
    Connection,
    Timeout,
    InterruptedBody,
    Tls,
    Dns,
    InvalidRequest,
    InvalidResponse,
    InvalidMetadata,
    LoginPage,
}
impl HttpFailureKind {
    pub(crate) fn retryable(self) -> bool {
        matches!(
            self,
            Self::Status(500 | 502 | 503 | 504)
                | Self::RateLimited(_)
                | Self::Connection
                | Self::Timeout
                | Self::InterruptedBody
        )
    }
    pub fn status(self) -> Option<u16> {
        match self {
            Self::Status(status) | Self::RateLimited(status) => Some(status),
            _ => None,
        }
    }
}
impl std::fmt::Display for HttpFailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code) => write!(f, "status code {code}"),
            Self::RateLimited(code) => write!(f, "status code {code} (rate limited)"),
            other => f.write_str(match other {
                Self::Connection => "transient connection failure",
                Self::Timeout => "request timed out",
                Self::InterruptedBody => "response body interrupted",
                Self::Tls => "TLS validation failed; check trusted CA configuration",
                Self::Dns => "DNS lookup failed",
                Self::InvalidRequest => "invalid URL, proxy or request configuration",
                Self::InvalidResponse => "invalid HTTP response or redirect",
                Self::InvalidMetadata => "invalid JSON metadata",
                Self::LoginPage => {
                    "server returned an HTML/login page instead of an asset or API response"
                }
                _ => unreachable!(),
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum AuthenticationDisposition {
    Absent,
    PublicBound,
    EnterpriseBound,
    OtherBound,
    EnterpriseUnbound { configured_host: Option<String> },
}

/// Contains only redacted locations and non-secret classifications, never headers.
#[derive(Debug)]
pub struct HttpFailure {
    pub(super) url: String,
    pub(super) effective_url: Option<String>,
    pub(super) kind: HttpFailureKind,
    pub(super) attempts: u8,
    pub(super) elapsed: Duration,
    pub(super) waited: Duration,
    pub(super) stop: RetryStopReason,
    pub(super) server_wait: Option<Duration>,
    pub(super) authentication: AuthenticationDisposition,
    pub(super) github_host: Option<String>,
    pub(super) api_host: Option<String>,
}
impl HttpFailure {
    pub(super) fn new(
        url: &str,
        kind: HttpFailureKind,
        budget: &RequestBudget,
        stop: RetryStopReason,
        authentication: AuthenticationDisposition,
    ) -> Self {
        Self {
            url: safe_url(url),
            effective_url: None,
            kind,
            attempts: budget.attempts,
            elapsed: Instant::now().saturating_duration_since(budget.started),
            waited: budget.waited,
            stop,
            server_wait: None,
            authentication,
            github_host: None,
            api_host: None,
        }
    }
    pub fn status(&self) -> Option<u16> {
        self.kind.status()
    }
    pub fn kind(&self) -> HttpFailureKind {
        self.kind
    }
    pub fn attempts(&self) -> u8 {
        self.attempts
    }
    pub fn stop_reason(&self) -> RetryStopReason {
        self.stop
    }
    pub(crate) fn github_context(&mut self, base: Option<&str>) {
        self.github_host =
            super::host_port(base.unwrap_or("https://api.github.com")).map(|(host, _)| host);
    }
    fn hint(&self) -> Option<String> {
        if !matches!(
            self.kind,
            HttpFailureKind::Status(401 | 403 | 404)
                | HttpFailureKind::RateLimited(_)
                | HttpFailureKind::LoginPage
        ) {
            return None;
        }
        let host = self.github_host.as_deref().or_else(|| {
            if self.url.starts_with("https://api.github.com/")
                || self.url.starts_with("https://github.com/")
            {
                Some("api.github.com")
            } else {
                None
            }
        })?;
        match &self.authentication {
            AuthenticationDisposition::PublicBound | AuthenticationDisposition::EnterpriseBound | AuthenticationDisposition::OtherBound =>
                Some("a credential was host-bound; check access, token validity/scopes, SSO and server rate-limit policy".into()),
            _ if matches!(host, "github.com" | "api.github.com") => Some(format!(
                "no GH_TOKEN/GITHUB_TOKEN is bound to github.com; {}authenticate or resolve from another environment",
                if matches!(self.kind, HttpFailureKind::RateLimited(_)) { "the anonymous API quota is 60 requests/hour per egress IP; " } else { "check repository access and rate-limit response headers; " })),
            _ if self.api_host.as_deref().is_some_and(|api| api != host) => Some(format!(
                "declared API host {host} differs from advertised API host {}; correct the URLs or explicitly bind credentials only to the intended host", self.api_host.as_deref().unwrap())),
            AuthenticationDisposition::EnterpriseUnbound { configured_host } => Some(format!(
                "enterprise token is present but not bound to {host} (GH_HOST/GITHUB_HOST: {}); set GH_HOST={host} only if this credential is intended for that host", configured_host.as_deref().unwrap_or("unset"))),
            AuthenticationDisposition::Absent => Some(format!(
                "no host-bound enterprise credential for {host}; set GH_HOST={host} and GH_ENTERPRISE_TOKEN/GITHUB_ENTERPRISE_TOKEN in the calling environment")),
        }
    }
}
impl std::fmt::Display for HttpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "http error fetching {}: {}; policy attempts {}; stop: {}; elapsed {:.3}s, retry wait {:.3}s",
            self.url,
            self.kind,
            self.attempts,
            self.stop,
            self.elapsed.as_secs_f64(),
            self.waited.as_secs_f64()
        )?;
        if let Some(url) = &self.effective_url {
            write!(f, "; effective URL {url}")?;
        }
        if let Some(wait) = self.server_wait {
            write!(f, "; server cooldown {:.0}s", wait.as_secs_f64())?;
        }
        if let Some(hint) = self.hint() {
            write!(f, "; {hint}")?;
        }
        Ok(())
    }
}
impl Error for HttpFailure {}

pub(crate) fn safe_url(value: &str) -> String {
    match url::Url::parse(value) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "<invalid URL>".into(),
    }
}

pub(crate) fn safe_location(value: &str) -> std::borrow::Cow<'_, str> {
    if url::Url::parse(value).is_ok_and(|url| url.host_str().is_some()) {
        std::borrow::Cow::Owned(safe_url(value))
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

pub(super) fn io_kind(error: &std::io::Error, body: bool) -> HttpFailureKind {
    if error
        .get_ref()
        .is_some_and(|inner| inner.is::<rustls::Error>())
    {
        return HttpFailureKind::Tls;
    }
    match error.kind() {
        std::io::ErrorKind::TimedOut => HttpFailureKind::Timeout,
        std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::Interrupted => {
            if body {
                HttpFailureKind::InterruptedBody
            } else {
                HttpFailureKind::Connection
            }
        }
        _ => HttpFailureKind::InvalidResponse,
    }
}

pub(super) fn transport_kind(error: &ureq::Transport) -> HttpFailureKind {
    let mut source = error.source();
    let mut io = None;
    while let Some(cause) = source {
        if cause.is::<rustls::Error>() {
            return HttpFailureKind::Tls;
        }
        if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            let kind = io_kind(error, false);
            if kind == HttpFailureKind::Tls {
                return kind;
            }
            io = Some(kind);
        }
        source = cause.source();
    }
    if let Some(kind) = io {
        return kind;
    }
    match error.kind() {
        ureq::ErrorKind::Dns => HttpFailureKind::Dns,
        ureq::ErrorKind::InvalidUrl
        | ureq::ErrorKind::UnknownScheme
        | ureq::ErrorKind::InvalidProxyUrl
        | ureq::ErrorKind::ProxyUnauthorized
        | ureq::ErrorKind::InsecureRequestHttpsOnly => HttpFailureKind::InvalidRequest,
        _ => HttpFailureKind::InvalidResponse,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_limit_advice_is_evidence_based_and_locations_are_redacted() {
        let now = Instant::now();
        let mut budget = RequestBudget::new(now);
        budget.begin(now).unwrap();
        let url = "https://user:PRIVATE-CANARY@api.github.com/repos/a/b?secret=PRIVATE-CANARY";
        let quota = HttpFailure::new(
            url,
            HttpFailureKind::RateLimited(403),
            &budget,
            RetryStopReason::WaitBudget,
            AuthenticationDisposition::Absent,
        );
        let text = quota.to_string();
        assert!(text.contains("GH_TOKEN/GITHUB_TOKEN") && text.contains("60 requests/hour"));
        assert!(!text.contains("PRIVATE-CANARY"));
        let forbidden = HttpFailure::new(
            url,
            HttpFailureKind::Status(403),
            &budget,
            RetryStopReason::NonRetryable,
            AuthenticationDisposition::Absent,
        );
        assert!(!forbidden.to_string().contains("60 requests/hour"));
        let mismatch = crate::FetchError::HashMismatch {
            url: url.into(),
            expected: "expected".into(),
            actual: "actual".into(),
        };
        assert!(!mismatch.to_string().contains("PRIVATE-CANARY"));
    }
    #[test]
    fn certificate_failures_are_not_transient_connections() {
        let error = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        );
        assert_eq!(io_kind(&error, false), HttpFailureKind::Tls);
        assert!(!io_kind(&error, false).retryable());
    }
}
