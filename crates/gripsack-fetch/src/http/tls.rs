//! Use the HTTP client's public TLS extension point without its bundled roots.
//! Certificate verification and handshaking remain entirely owned by Rustls.

use std::{
    fmt,
    io::{self, Read, Write},
    net::TcpStream,
    sync::Arc,
};

pub(super) struct Connector {
    pub(super) config: Arc<rustls::ClientConfig>,
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
