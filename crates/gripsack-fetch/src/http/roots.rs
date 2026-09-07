//! Snapshot certificate locations without reading them. Loading happens on first
//! network use; no later repo environment can alter a provisioning context.

use std::path::PathBuf;

pub(super) struct Locations {
    file: Option<PathBuf>,
    directories: Vec<PathBuf>,
}

impl Locations {
    pub(super) fn capture() -> Self {
        Self {
            file: std::env::var_os("SSL_CERT_FILE").map(PathBuf::from),
            directories: std::env::var_os("SSL_CERT_DIR")
                .map(|paths| std::env::split_paths(&paths).collect())
                .unwrap_or_default(),
        }
    }

    pub(super) fn load(&self, roots: &mut rustls::RootCertStore) {
        if self.file.is_some() || !self.directories.is_empty() {
            append(roots, self.file.as_deref(), None);
            for directory in &self.directories {
                append(roots, None, Some(directory));
            }
        } else {
            system(roots);
        }
    }
}

fn append(
    roots: &mut rustls::RootCertStore,
    file: Option<&std::path::Path>,
    directory: Option<&std::path::Path>,
) {
    let result = rustls_native_certs::load_certs_from_paths(file, directory);
    if !result.errors.is_empty() {
        tracing::warn!(
            errors = result.errors.len(),
            "some system CA roots failed to load"
        );
    }
    roots.add_parsable_certificates(result.certs);
}

#[cfg(not(target_os = "macos"))]
fn system(roots: &mut rustls::RootCertStore) {
    // rustls/openssl-probe's Linux bundle order, without its ambient-env lookup.
    const BUNDLES: &[&str] = &[
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/ca-bundle.pem",
        "/etc/pki/tls/cacert.pem",
        "/etc/ssl/cert.pem",
        "/opt/etc/ssl/certs/ca-certificates.crt",
        "/etc/ssl/certs/cacert.pem",
    ];
    let file = BUNDLES
        .iter()
        .map(std::path::Path::new)
        .find(|path| path.is_file());
    append(roots, file, None);
    for directory in [
        "/etc/ssl/certs",
        "/etc/pki/tls/certs",
        "/etc/security/certificates",
    ] {
        let directory = std::path::Path::new(directory);
        if directory.is_dir() {
            append(roots, None, Some(directory));
        }
    }
}

#[cfg(target_os = "macos")]
fn system(roots: &mut rustls::RootCertStore) {
    use security_framework::trust_settings::{Domain, TrustSettings, TrustSettingsForCertificate};
    // Native trust precedence matches rustls-native-certs: user > admin > system.
    // Call the OS API directly because its public loader re-reads SSL_CERT_*.
    let mut certificates = std::collections::HashMap::new();
    for domain in [Domain::User, Domain::Admin, Domain::System] {
        let settings = TrustSettings::new(domain);
        let iterator = match settings.iter() {
            Ok(iterator) => iterator,
            Err(_) => {
                tracing::warn!("system CA trust domain could not be read");
                continue;
            }
        };
        for certificate in iterator {
            let trust = match settings.tls_trust_settings_for_certificate(&certificate) {
                Ok(trust) => trust.unwrap_or(TrustSettingsForCertificate::TrustRoot),
                Err(_) => {
                    tracing::warn!("system CA trust settings could not be read");
                    continue;
                }
            };
            certificates.entry(certificate.to_der()).or_insert(trust);
        }
    }
    roots.add_parsable_certificates(certificates.into_iter().filter_map(|(der, trust)| {
        matches!(
            trust,
            TrustSettingsForCertificate::TrustRoot | TrustSettingsForCertificate::TrustAsRoot
        )
        .then(|| rustls::pki_types::CertificateDer::from(der))
    }));
}
