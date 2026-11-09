//! Minimal HTTP for release downloads: rustls with the system roots (no
//! openssl), bounded size, a timeout, and `HTTPS_PROXY` for https URLs.

use sha2::{Digest, Sha256};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

fn agent(redirects: u32) -> Result<ureq::Agent, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut builder = ureq::AgentBuilder::new()
        .tls_connector(Arc::new(Connector {
            config: Arc::new(config),
        }))
        .redirects(redirects)
        .timeout_connect(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .try_proxy_from_env(false);
    let proxy = ["HTTPS_PROXY", "https_proxy"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .and_then(|url| ureq::Proxy::new(url).ok());
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy);
    }
    Ok(builder.build())
}

fn get(agent: &ureq::Agent, url: &str) -> Result<ureq::Response, String> {
    agent
        .get(url)
        .set("User-Agent", "gripsack")
        .call()
        .map_err(|e| format!("GET {url}: {e}"))
}

/// The `Location` of `url`'s redirect, resolved against `url`.
pub(super) fn redirect_location(url: &str) -> Result<String, String> {
    let response = get(&agent(0)?, url)?;
    let location = response
        .header("location")
        .filter(|_| (300..400).contains(&response.status()))
        .ok_or_else(|| format!("{url} did not redirect"))?;
    let base = url::Url::parse(url).map_err(|e| e.to_string())?;
    base.join(location)
        .map(String::from)
        .map_err(|e| e.to_string())
}

/// Download at most `limit` bytes to a temporary file; returns it with its SHA-256.
pub(super) fn download(url: &str, limit: u64) -> Result<(tempfile::NamedTempFile, String), String> {
    let response = get(&agent(5)?, url)?;
    let mut reader = response.into_reader().take(limit + 1);
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| format!("GET {url}: {e}"))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > limit {
            return Err(format!("{url} exceeds the {limit}-byte download limit"));
        }
        hasher.update(&buffer[..n]);
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
    }
    file.flush().map_err(|e| e.to_string())?;
    let digest = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((file, digest))
}

struct Connector {
    config: Arc<rustls::ClientConfig>,
}

impl ureq::TlsConnector for Connector {
    fn connect(
        &self,
        name: &str,
        mut transport: Box<dyn ureq::ReadWrite>,
    ) -> Result<Box<dyn ureq::ReadWrite>, ureq::Error> {
        let name = name
            .strip_prefix('[')
            .and_then(|name| name.strip_suffix(']'))
            .unwrap_or(name);
        let server = rustls::pki_types::ServerName::try_from(name)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
            .to_owned();
        let mut connection = rustls::ClientConnection::new(Arc::clone(&self.config), server)
            .map_err(io::Error::other)?;
        // Verify the peer before any HTTP bytes are sent. The original I/O
        // error chain preserves certificate failures and transport timeouts.
        connection.complete_io(&mut transport)?;
        Ok(Box::new(Stream {
            inner: rustls::StreamOwned::new(connection, transport),
        }))
    }
}

struct Stream {
    inner: rustls::StreamOwned<rustls::ClientConnection, Box<dyn ureq::ReadWrite>>,
}

impl fmt::Debug for Stream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never expose TLS connection/session state through transport diagnostics.
        formatter.write_str("RustlsStream")
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl ureq::ReadWrite for Stream {
    fn socket(&self) -> Option<&TcpStream> {
        // The client must retain access to the socket to enforce its original
        // connect/read/write deadline across handshake and response streaming.
        self.inner.sock.socket()
    }
}
