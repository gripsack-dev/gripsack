//! Native HTTP effects share the same explicit rustls crypto provider.
//! Reqwest's no-provider feature requires initialization even for Unix HTTP.
pub(crate) fn builder() -> reqwest::blocking::ClientBuilder {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // A failed installation means another caller initialized the global
        // provider first; it is not a missing-provider or TLS-validation fallback.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::blocking::Client::builder()
}
