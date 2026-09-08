//! Per-context connection pools and network policy (0042).
//! No client is global: provisioning and artifact contexts capture different
//! environment phases, and a subsequent command can use new proxy/CA settings.

mod body;
mod failure;
mod request;
mod retry;
mod roots;
pub(crate) use failure::safe_location;
pub use failure::{HttpFailure, HttpFailureKind};
pub(crate) use request::RequestKind;
pub use retry::RetryStopReason;

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub(crate) struct Client {
    agents: OnceLock<(ureq::Agent, ureq::Agent)>,
    certificates: roots::Locations,
    proxy: Option<ureq::Proxy>,
    policy: Policy,
    cooldowns: Mutex<std::collections::BTreeMap<String, request::Cooldown>>,
}

/// Deliberately not Debug: authentication material never enters tracing.
struct Policy {
    no_proxy: String,
    github: Option<String>,
    enterprise_host: Option<String>,
    enterprise: Option<String>,
}

impl Policy {
    fn from_env() -> Self {
        let token = |primary, fallback| {
            std::env::var(primary)
                .or_else(|_| std::env::var(fallback))
                .ok()
                .filter(|token| !token.trim().is_empty())
                .map(|t| format!("Bearer {t}"))
        };
        Self {
            no_proxy: std::env::var("NO_PROXY")
                .or_else(|_| std::env::var("no_proxy"))
                .unwrap_or_default(),
            github: token("GITHUB_TOKEN", "GH_TOKEN"),
            enterprise_host: std::env::var("GH_HOST")
                .or_else(|_| std::env::var("GITHUB_HOST"))
                .ok()
                .map(|host| host.trim().to_ascii_lowercase()),
            enterprise: token("GITHUB_ENTERPRISE_TOKEN", "GH_ENTERPRISE_TOKEN"),
        }
    }

    fn header(&self, url: &str) -> Option<&str> {
        let (host, _) = host_port(url)?;
        if host == "github.com" || host == "api.github.com" {
            self.github.as_deref()
        } else if self.enterprise_host.as_deref() == Some(host.as_str()) {
            self.enterprise.as_deref()
        } else {
            None
        }
    }

    fn authentication(&self, url: &str) -> failure::AuthenticationDisposition {
        use failure::AuthenticationDisposition;
        let host = host_port(url).map(|(host, _)| host);
        if host
            .as_deref()
            .is_some_and(|host| matches!(host, "github.com" | "api.github.com"))
        {
            if self.github.is_some() {
                AuthenticationDisposition::PublicBound
            } else {
                AuthenticationDisposition::Absent
            }
        } else if self.header(url).is_some() {
            AuthenticationDisposition::EnterpriseBound
        } else if self.enterprise.is_some() {
            AuthenticationDisposition::EnterpriseUnbound {
                configured_host: self.enterprise_host.clone(),
            }
        } else {
            AuthenticationDisposition::Absent
        }
    }

    fn bypasses(&self, url: &str) -> bool {
        let Some((host, port)) = host_port(url) else {
            return false;
        };
        self.no_proxy
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .any(|entry| entry_matches(entry, &host, port.as_deref()))
    }
}

impl Client {
    pub(crate) fn from_env() -> Self {
        let proxy = [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .and_then(|url| ureq::Proxy::new(url).ok())
        });
        Self {
            agents: OnceLock::new(),
            certificates: roots::Locations::capture(),
            proxy,
            policy: Policy::from_env(),
            cooldowns: Mutex::new(Default::default()),
        }
    }

    fn request(&self, url: &str) -> ureq::Request {
        let (direct, proxied) = self.agents.get_or_init(|| {
            let tls = tls_config(&self.certificates);
            let builder = || {
                ureq::AgentBuilder::new()
                    .tls_config(Arc::clone(&tls))
                    .timeout_connect(Duration::from_secs(30))
                    .timeout(Duration::from_secs(600))
                    .try_proxy_from_env(false)
                    .redirect_auth_headers(ureq::RedirectAuthHeaders::SameHost)
            };
            let direct = builder().build();
            let proxied = match &self.proxy {
                Some(proxy) => builder().proxy(proxy.clone()).build(),
                None => direct.clone(),
            };
            (direct, proxied)
        });
        if self.policy.bypasses(url) {
            direct.get(url)
        } else {
            proxied.get(url)
        }
    }
}

/// Use the same URL parser as the transport. Splitting authority text by hand
/// can attach a token to a different host when backslashes/userinfo normalize.
pub(crate) fn host_port(value: &str) -> Option<(String, Option<String>)> {
    let url = url::Url::parse(value).ok()?;
    let host = url
        .host_str()?
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    Some((host, url.port().map(|port| port.to_string())))
}

fn entry_matches(entry: &str, host: &str, port: Option<&str>) -> bool {
    if entry == "*" {
        return true;
    }
    let (entry_host, entry_port) = match entry.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => (host, Some(port)),
        _ => (entry, None),
    };
    if entry_port.is_some_and(|wanted| Some(wanted) != port) {
        return false;
    }
    let entry_host = entry_host.trim_start_matches('.').to_ascii_lowercase();
    host == entry_host
        || host
            .strip_suffix(&entry_host)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn tls_config(locations: &roots::Locations) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    locations.load(&mut roots);
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy {
            no_proxy: ".internal:81,localhost".into(),
            github: Some("Bearer public".into()),
            enterprise_host: Some("ghe.internal".into()),
            enterprise: Some("Bearer enterprise".into()),
        }
    }

    #[test]
    fn host_binding_survives_url_normalization() {
        let policy = policy();
        assert_eq!(
            policy.header("https://api.github.com/repos/a"),
            Some("Bearer public")
        );
        assert_eq!(
            policy.header("https://ghe.internal/api/v3"),
            Some("Bearer enterprise")
        );
        assert!(policy.header("https://evil.invalid/archive").is_none());
        assert!(
            policy
                .header("https://evil.invalid\\@github.com/archive")
                .is_none()
        );
        assert!(
            policy
                .header("https://github.com.evil.invalid/archive")
                .is_none()
        );
        let mut unbound = policy;
        unbound.enterprise_host = None;
        assert!(unbound.header("https://ghe.internal/archive").is_none());
    }

    #[test]
    fn credential_routing_model_uses_actual_host_selector() {
        for public in [false, true] {
            for enterprise in [false, true] {
                for bound in [None, Some("enterprise-one"), Some("enterprise-two")] {
                    let policy = Policy {
                        no_proxy: String::new(),
                        github: public.then(|| "public-canary".into()),
                        enterprise: enterprise.then(|| "enterprise-canary".into()),
                        enterprise_host: bound.map(str::to_string),
                    };
                    for host in [
                        "github.com",
                        "api.github.com",
                        "enterprise-one",
                        "enterprise-two",
                    ] {
                        let expected = if matches!(host, "github.com" | "api.github.com") {
                            public.then_some("public-canary")
                        } else if Some(host) == bound {
                            enterprise.then_some("enterprise-canary")
                        } else {
                            None
                        };
                        assert_eq!(policy.header(&format!("https://{host}/resource")), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn proxy_bypass_preserves_domain_and_port_boundaries() {
        let policy = policy();
        assert!(policy.bypasses("http://sub.internal:81/a"));
        assert!(!policy.bypasses("http://sub.internal:82/a"));
        assert!(!policy.bypasses("http://notinternal:81/a"));
        assert!(policy.bypasses("http://localhost/a"));
        assert_eq!(
            host_port("http://[::1]:8080/a"),
            Some(("::1".into(), Some("8080".into())))
        );
    }
}
