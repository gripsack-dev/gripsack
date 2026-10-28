//! Snapshot certificate locations without reading them. Loading happens on first
//! network use; no later repo environment can alter a provisioning context.

use std::path::PathBuf;

pub(super) struct Locations {
    file: Option<PathBuf>,
    directories: Vec<PathBuf>,
}

impl Locations {
    pub(super) fn capture(build_env: &crate::build_env::BuildProcessEnv) -> Self {
        Self {
            file: build_env.var_os("SSL_CERT_FILE").map(PathBuf::from),
            directories: build_env
                .var_os("SSL_CERT_DIR")
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
