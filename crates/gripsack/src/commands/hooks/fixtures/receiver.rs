//! Harmless fixture receiver: token and effect are one atomically replaced
//! private record, committed before HTTP success. This is not a generic service.
use super::{FixtureRoot, invalid};
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

const REQUEST: &[u8] =
    b"POST /effect HTTP/1.1\r\nHost: localhost\r\nContent-Length: 64\r\nConnection: close\r\n\r\n";
const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReceiverState {
    tokens: BTreeSet<String>,
    effects: u64,
}

pub(super) struct Receiver {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<io::Result<()>>>,
}
impl Receiver {
    pub(super) fn start(root: &FixtureRoot) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let directory = gripsack_fs::open(&root.path)?;
        let thread = std::thread::spawn(move || {
            let mut state = read_state(&directory)?;
            while !stopping.load(Ordering::Acquire) {
                let (mut connection, peer) = listener.accept()?;
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                if !peer.ip().is_loopback() {
                    continue;
                }
                connection.set_read_timeout(Some(Duration::from_secs(2)))?;
                connection.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut header = [0; REQUEST.len()];
                if connection.read_exact(&mut header).is_err() || header != REQUEST {
                    continue;
                }
                let mut token = [0; 64];
                if connection.read_exact(&mut token).is_err() {
                    continue;
                }
                let Ok(token) = std::str::from_utf8(&token) else {
                    continue;
                };
                if Sha256Digest::parse(token).is_err() {
                    continue;
                }
                if state.tokens.insert(token.to_owned()) {
                    if state.tokens.len() > 16 {
                        return Err(invalid("fixture token budget exhausted"));
                    }
                    state.effects = state
                        .effects
                        .checked_add(1)
                        .ok_or_else(|| invalid("fixture effect counter overflow"))?;
                    let bytes = serde_json::to_vec(&state).map_err(io::Error::other)?;
                    gripsack_fs::atomic_write_with_mode(
                        &directory,
                        Path::new("receiver.json"),
                        &bytes,
                        0o600,
                    )?;
                }
                // Failure after commit is deliberately ambiguous to the caller;
                // another request with this token observes the same one effect.
                let _ = connection.write_all(RESPONSE);
            }
            Ok(())
        });
        Ok(Self {
            port,
            stop,
            thread: Some(thread),
        })
    }
    pub(super) fn port(&self) -> u16 {
        self.port
    }
    pub(super) fn finish(mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, self.port));
        self.thread
            .take()
            .ok_or_else(|| invalid("fixture receiver already joined"))?
            .join()
            .map_err(|_| invalid("fixture receiver panicked"))?
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn notify(port: u16, intent: Sha256Digest) -> io::Result<()> {
    let mut connection = TcpStream::connect_timeout(
        &std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port)),
        Duration::from_secs(2),
    )?;
    connection.set_read_timeout(Some(Duration::from_secs(2)))?;
    connection.set_write_timeout(Some(Duration::from_secs(2)))?;
    connection.write_all(REQUEST)?;
    write!(connection, "{intent}")?;
    let mut response = [0; RESPONSE.len()];
    connection.read_exact(&mut response)?;
    if response != RESPONSE {
        return Err(invalid(
            "fixture receiver did not acknowledge its durable effect",
        ));
    }
    Ok(())
}

pub(super) fn effects(root: &FixtureRoot) -> io::Result<u64> {
    Ok(read_state(&root.directory)?.effects)
}

fn read_state(directory: &gripsack_fs::Dir) -> io::Result<ReceiverState> {
    let mut file = match gripsack_fs::open_file_nofollow(directory, Path::new("receiver.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ReceiverState::default());
        }
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    (&mut file).take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(invalid("fixture receiver record exceeds its budget"));
    }
    let state: ReceiverState = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if state.tokens.len() > 16
        || state.effects != state.tokens.len() as u64
        || state
            .tokens
            .iter()
            .any(|token| Sha256Digest::parse(token).is_err())
    {
        return Err(invalid("fixture token/effect atomicity was lost"));
    }
    gripsack_fs::fault::operation(
        gripsack_fs::fault::Boundary::FileSync,
        Path::new("receiver.json"),
        || file.sync_all(),
    )?;
    gripsack_fs::fsync_dir(directory, Path::new("."))?;
    Ok(state)
}
