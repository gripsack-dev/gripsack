//! The one HTTP client every network path shares (fetch + resolve).
//!
//! Two environment behaviors ureq does not give you by default:
//!
//! - **Proxy** — `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` (either case) are
//!   honored; without this every fetch is dead behind a corporate proxy.
//!   (`no_proxy` is not supported by ureq 2.x — known limitation.)
//! - **Roots** — trust is the bundled webpki roots *plus* the system
//!   roots. rustls-native-certs goes through openssl-probe on Linux, so
//!   `SSL_CERT_FILE`/`SSL_CERT_DIR` are honored — that is what makes a
//!   TLS-intercepting proxy's CA verifiable.

use std::sync::Arc;

/// A ureq agent configured for the environment it runs in.
pub(crate) fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .tls_config(tls_config())
        .build()
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    // Bundled roots first: minimal containers without a CA store keep
    // working. System roots on top: intercepting proxies verify.
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    match rustls_native_certs::load_native_certs() {
        Ok(certs) => {
            roots.add_parsable_certificates(certs);
        }
        Err(e) => tracing::warn!("could not load system CA roots: {e}"),
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}
