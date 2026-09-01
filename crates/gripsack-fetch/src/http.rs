//! The one HTTP client every network path shares (fetch + resolve).
//!
//! Two environment behaviors ureq does not give you by default:
//!
//! - **Proxy** — `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` (either case) are
//!   honored; without this every fetch is dead behind a corporate proxy.
//! - **Roots** — trust is the bundled webpki roots *plus* the system
//!   roots. rustls-native-certs goes through openssl-probe on Linux, so
//!   `SSL_CERT_FILE`/`SSL_CERT_DIR` are honored — that is what makes a
//!   TLS-intercepting proxy's CA verifiable.
//!
//! Plus the one ureq 2.x never grew: `NO_PROXY`/`no_proxy` (curl
//! semantics — comma-separated, `*` matches everything, a bare domain
//! matches it and its subdomains, optional `:port`).

use std::sync::Arc;

/// The auth header for a URL, if a token is bound to its host. The gh
/// CLI convention, because it is the ecosystem standard:
/// `GH_TOKEN`/`GITHUB_TOKEN` only ever go to github.com hosts;
/// `GH_ENTERPRISE_TOKEN`/`GITHUB_ENTERPRISE_TOKEN` only to the ONE
/// enterprise host `GH_HOST`/`GITHUB_HOST` names. A token is NEVER
/// attached outside its binding — a mixed repo (some modules on GHE,
/// most on public github) must not leak either credential to the
/// other side (enterprise review finding). "Any non-github host"
/// cannot be the enterprise binding here, unlike in gh: one gripsack
/// run fetches from every host the modules name.
pub(crate) fn auth_header(url: &str) -> Option<String> {
    let (host, _) = host_port(url)?;
    let github_host = host == "api.github.com" || host == "github.com";
    let (primary, fallback) = if github_host {
        ("GITHUB_TOKEN", "GH_TOKEN")
    } else {
        let bound = std::env::var("GH_HOST")
            .or_else(|_| std::env::var("GITHUB_HOST"))
            .ok()?
            .trim()
            .to_lowercase();
        if host != bound {
            return None;
        }
        ("GITHUB_ENTERPRISE_TOKEN", "GH_ENTERPRISE_TOKEN")
    };
    std::env::var(primary)
        .or_else(|_| std::env::var(fallback))
        .ok()
        .map(|token| format!("Bearer {token}"))
}

/// GET `url` through the right agent for the environment: direct when
/// no_proxy says so, the env-proxy agent otherwise.
///
/// The one choke point for every network path (fetch + resolve) —
/// and therefore where rate budgets are enforced: the URL's host
/// acquires a token before the request is built (0002 §throttle).
pub(crate) fn get(url: &str) -> ureq::Request {
    crate::throttle::acquire_url(url);
    if proxy_bypassed(url) {
        direct_agent().get(url)
    } else {
        agent().get(url)
    }
}

/// A ureq agent configured for the environment it runs in.
pub(crate) fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .tls_config(tls_config())
        // ureq's default strips Authorization on EVERY redirect; a
        // transferred repo (301 repos/<name> → repositories/<id>)
        // then silently re-requests anonymously into the rate-limited
        // pool. SameHost keeps the token on its bound host only —
        // cross-host still strips, so the no-leak rule survives.
        .redirect_auth_headers(ureq::RedirectAuthHeaders::SameHost)
        .build()
}

/// An agent that never proxies.
fn direct_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .tls_config(tls_config())
        .redirect_auth_headers(ureq::RedirectAuthHeaders::SameHost)
        .build()
}

/// curl-style no_proxy: does this URL bypass the proxy?
fn proxy_bypassed(url: &str) -> bool {
    let list = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    if list.trim().is_empty() {
        return false;
    }
    let Some((host, port)) = host_port(url) else {
        return false;
    };
    list.split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .any(|entry| entry_matches(entry, &host, port.as_deref()))
}

/// (host, port) out of a URL — scheme://[user@]host[:port][/...].
/// Returns lowercase host; port as the explicit string if present.
fn host_port(url: &str) -> Option<(String, Option<String>)> {
    let (_, after_scheme) = url.split_once("://")?;
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next()?;
    let hostport = hostport.strip_prefix('[').unwrap_or(hostport);
    let (host, port) = match hostport.split_once(']') {
        Some((h, rest)) => (h, rest.strip_prefix(':').map(str::to_string)),
        None => match hostport.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p.to_string())),
            _ => (hostport, None),
        },
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_lowercase(), port))
}

fn entry_matches(entry: &str, host: &str, port: Option<&str>) -> bool {
    if entry == "*" {
        return true;
    }
    let (entry_host, entry_port) = match entry.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
        _ => (entry, None),
    };
    if let Some(p) = entry_port
        && port != Some(p)
    {
        return false;
    }
    let entry_host = entry_host.trim_start_matches('.').to_lowercase();
    host == entry_host || host.ends_with(&format!(".{entry_host}"))
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    // Bundled roots first: minimal containers without a CA store keep
    // working. System roots on top: intercepting proxies verify.
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // 0.8 API: partial success is normal — take what loaded, log the rest.
    let native = rustls_native_certs::load_native_certs();
    if !native.errors.is_empty() {
        tracing::warn!(
            "some system CA roots failed to load ({} errors)",
            native.errors.len()
        );
    }
    roots.add_parsable_certificates(native.certs);
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parses_common_shapes() {
        assert_eq!(
            host_port("https://github.com/org/repo"),
            Some(("github.com".into(), None))
        );
        assert_eq!(
            host_port("https://user:pw@proxy.internal:81/x"),
            Some(("proxy.internal".into(), Some("81".into())))
        );
        assert_eq!(
            host_port("http://[::1]:8080/y"),
            Some(("::1".into(), Some("8080".into())))
        );
        assert_eq!(host_port("not a url at all"), None);
    }

    #[test]
    fn entry_matching_is_curl_shaped() {
        assert!(entry_matches("*", "anything", None));
        assert!(entry_matches("example.com", "example.com", None));
        assert!(entry_matches("example.com", "a.example.com", None));
        assert!(entry_matches(".example.com", "a.example.com", None));
        assert!(!entry_matches("example.com", "notexample.com", None));
        assert!(entry_matches("internal:81", "internal", Some("81")));
        assert!(!entry_matches("internal:81", "internal", Some("82")));
        assert!(entry_matches("INTERNAL", "internal", None));
    }

    #[test]
    fn bypass_reads_the_env() {
        unsafe {
            std::env::set_var("NO_PROXY", "github.com, .internal:81");
        }
        assert!(proxy_bypassed("https://github.com/x"));
        assert!(proxy_bypassed("https://api.github.com/x"));
        assert!(proxy_bypassed("https://proxy.internal:81/x"));
        assert!(!proxy_bypassed("https://proxy.internal:82/x"));
        assert!(!proxy_bypassed("https://crates.io/x"));
        unsafe {
            std::env::remove_var("NO_PROXY");
        }
        assert!(!proxy_bypassed("https://github.com/x"));
    }
}

#[cfg(test)]
mod auth_tests {
    use super::auth_header;
    // env mutation races other env tests in this file — one lock for all
    static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    fn with_env(vars: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _g = LOCK.lock();
        let saved: Vec<_> = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var(k).ok()))
            .collect();
        for (k, v) in vars {
            unsafe {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
        f();
        for (k, old) in saved {
            unsafe {
                match old {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    const ALL: [&str; 6] = [
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GH_HOST",
        "GITHUB_HOST",
    ];

    #[test]
    fn github_token_only_binds_github_hosts() {
        with_env(
            &[
                (ALL[0], Some("secret")),
                (ALL[1], None),
                (ALL[2], None),
                (ALL[3], None),
            ],
            || {
                assert_eq!(
                    auth_header("https://api.github.com/repos/x"),
                    Some("Bearer secret".into())
                );
                assert!(auth_header("https://ghe.corp.example/api/v3/repos/x").is_none());
            },
        );
    }

    #[test]
    fn enterprise_token_binds_only_gh_host() {
        with_env(
            &[
                (ALL[0], None),
                (ALL[1], None),
                (ALL[2], Some("corp")),
                (ALL[3], None),
                (ALL[4], Some("ghe.corp.example")),
                (ALL[5], None),
            ],
            || {
                assert_eq!(
                    auth_header("https://ghe.corp.example/api/v3/repos/x"),
                    Some("Bearer corp".into())
                );
                assert!(auth_header("https://api.github.com/repos/x").is_none());
                // the leak this guard exists for: a module tarball on a
                // third-party host must never see the enterprise token
                assert!(auth_header("https://evil.example/tools.tar.gz").is_none());
            },
        );
    }

    #[test]
    fn enterprise_token_without_gh_host_attaches_nowhere() {
        with_env(
            &[
                (ALL[0], None),
                (ALL[1], None),
                (ALL[2], Some("corp")),
                (ALL[3], None),
                (ALL[4], None),
                (ALL[5], None),
            ],
            || {
                // no binding named → we cannot know the enterprise
                // host; fail safe rather than exfiltrate
                assert!(auth_header("https://ghe.corp.example/api/v3/repos/x").is_none());
                assert!(auth_header("https://evil.example/tools.tar.gz").is_none());
            },
        );
    }

    #[test]
    fn no_token_no_header() {
        with_env(&ALL.map(|k| (k, None)), || {
            assert!(auth_header("https://api.github.com/repos/x").is_none());
            assert!(auth_header("https://ghe.corp.example/api/v3/repos/x").is_none());
        });
    }
}
