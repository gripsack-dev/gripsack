//! The core side of the bridge transport: spawn the bridge binary,
//! speak the framed protocol over its stdio, and drive each session
//! through the [`EventGate`] fence until exactly one terminal. The
//! driver is transport-only — it never trusts the bridge: version
//! negotiation is exact, every event is fenced, logs are bounded by
//! the gate, and a session that ends without a terminal is an error,
//! not a success.

use crate::protocol::{
    self, Digest, EventGate, FromBridge, PROTOCOL_VERSION, ProtocolError, SessionId, Terminal,
    ToBridge,
};
use std::io::{BufReader, Write};
use std::process::{Child, Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("bridge process failed: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("bridge protocol violation: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("bridge wire error: {0}")]
    Wire(#[source] std::io::Error),
    #[error("session {0:?} ended without a terminal event")]
    NoTerminal(String),
    #[error("session {0:?} exceeded {1} events without a terminal")]
    EventBudget(String, u64),
    #[error("bridge did not negotiate: first reply was {0:?}")]
    NotNegotiated(Box<FromBridge>),
}

/// One live bridge child with its framed stdio.
pub struct BridgeProcess {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    negotiated: bool,
}

/// How many bridge→core events one session may produce before the
/// core gives up — a bridge stuck in an event loop is a failure, not
/// a hang.
const SESSION_EVENT_BUDGET: u64 = 8192;

impl BridgeProcess {
    pub fn spawn(mut command: Command) -> Result<Self, TransportError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(TransportError::Spawn)?;
        let reader = BufReader::new(child.stdout.take().expect("piped stdout"));
        Ok(Self {
            child,
            reader,
            negotiated: false,
        })
    }

    fn send(&mut self, message: &ToBridge) -> Result<(), TransportError> {
        let frame = protocol::encode_frame(message);
        self.child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(&frame)
            .map_err(TransportError::Wire)?;
        self.child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .flush()
            .map_err(TransportError::Wire)
    }

    fn receive(&mut self) -> Result<FromBridge, TransportError> {
        let mut header = [0u8; 8];
        std::io::Read::read_exact(&mut self.reader, &mut header).map_err(TransportError::Wire)?;
        let declared = u64::from_le_bytes(header);
        if declared > protocol::MAX_FRAME_BYTES as u64 {
            return Err(ProtocolError::OversizedFrame(declared, protocol::MAX_FRAME_BYTES).into());
        }
        let mut body = vec![0u8; declared as usize];
        std::io::Read::read_exact(&mut self.reader, &mut body).map_err(TransportError::Wire)?;
        let message: FromBridge =
            serde_json::from_slice(&body).map_err(ProtocolError::Malformed)?;
        Ok(message)
    }

    /// Negotiate exactly once, refusing every other version.
    pub fn negotiate(&mut self) -> Result<Vec<String>, TransportError> {
        if self.negotiated {
            return Err(TransportError::Protocol(ProtocolError::NotNegotiate(
                "double negotiation".into(),
            )));
        }
        self.send(&ToBridge::Negotiate {
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec!["protocol.v1".into()],
        })?;
        let reply = self.receive()?;
        if !matches!(reply, FromBridge::NegotiateOk { .. }) {
            return Err(TransportError::NotNegotiated(Box::new(reply)));
        }
        let capabilities = protocol::negotiate(&reply)?;
        self.negotiated = true;
        Ok(capabilities)
    }

    /// Submit a session and drive it to exactly one terminal. Every
    /// event is fenced by the session gate; the returned terminal is
    /// what the bridge proved, never a default.
    pub fn submit(
        &mut self,
        session: &SessionId,
        definition: &Digest,
        definition_len: u64,
        exporter: &Digest,
    ) -> Result<Terminal, TransportError> {
        if !self.negotiated {
            return Err(TransportError::Protocol(ProtocolError::NotNegotiate(
                "submit before negotiation".into(),
            )));
        }
        self.send(&ToBridge::Submit {
            session: session.clone(),
            definition: definition.clone(),
            definition_len,
            exporter: exporter.clone(),
        })?;
        let mut gate: Option<(u64, EventGate)> = None;
        let mut events = 0u64;
        loop {
            let reply = self.receive()?;
            events += 1;
            if events > SESSION_EVENT_BUDGET {
                return Err(TransportError::EventBudget(
                    session.as_str().to_owned(),
                    events,
                ));
            }
            let (epoch, mut event_gate) = match (&reply, gate.take()) {
                (FromBridge::Accepted { epoch, .. }, None) => (*epoch, EventGate::new(*epoch)),
                (_, Some(live)) => live,
                (other, None) => {
                    return Err(TransportError::Protocol(ProtocolError::NotNegotiate(
                        format!("{other:?} before the session was accepted"),
                    )));
                }
            };
            match reply {
                FromBridge::Accepted { .. } | FromBridge::NegotiateOk { .. } => {}
                FromBridge::Event { kind, .. } => match kind {
                    protocol::BridgeEvent::Log { .. } => map_gate(event_gate.log(epoch))?,
                    protocol::BridgeEvent::Progress { .. } => map_gate(event_gate.observe(epoch))?,
                },
                FromBridge::Exported { .. } => {
                    map_gate(event_gate.observe(epoch))?;
                }
                FromBridge::Done { .. } => {
                    map_gate(event_gate.terminal(epoch, Terminal::Done))?;
                    return Ok(Terminal::Done);
                }
                FromBridge::Failed { .. } => {
                    map_gate(event_gate.terminal(epoch, Terminal::Failed))?;
                    return Ok(Terminal::Failed);
                }
                FromBridge::Cancelled { .. } => {
                    map_gate(event_gate.terminal(epoch, Terminal::Cancelled))?;
                    return Ok(Terminal::Cancelled);
                }
            }
            gate = Some((epoch, event_gate));
        }
    }

    /// Idempotent cancellation: an unknown or finished session still
    /// acknowledges, and the core never treats the ack as success of
    /// the cancelled work.
    pub fn cancel(&mut self, session: &SessionId) -> Result<(), TransportError> {
        if !self.negotiated {
            return Err(TransportError::Protocol(ProtocolError::NotNegotiate(
                "cancel before negotiation".into(),
            )));
        }
        self.send(&ToBridge::Cancel {
            session: session.clone(),
        })?;
        let reply = self.receive()?;
        match reply {
            FromBridge::Cancelled { .. } => Ok(()),
            other => Err(TransportError::Protocol(ProtocolError::NotNegotiate(
                format!("expected Cancelled, got {other:?}"),
            ))),
        }
    }
}

impl Drop for BridgeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn map_gate(result: Result<(), protocol::GateError>) -> Result<(), TransportError> {
    result.map_err(|error| {
        TransportError::Protocol(ProtocolError::NotNegotiate(format!(
            "session fence: {error}"
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge_bin() -> std::path::PathBuf {
        std::env::var_os("GRIPSACK_BRIDGE_BIN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tools/buildkit-bridge/bridge-bin")
            })
    }

    /// Cross-process, cross-language: the Rust core drives the REAL
    /// compiled Go bridge end to end over stdio. Ignored in the
    /// standard gate because `tools/` sits outside the test image;
    /// run explicitly via `cargo test -p gripsack-buildkit -- --ignored`
    /// after `tools/buildkit-bridge/build.sh` (the receipt records the
    /// explicit run).
    #[test]
    #[ignore = "requires tools/buildkit-bridge/bridge-bin (built by the pinned golang gate)"]
    fn the_real_go_bridge_negotiates_fails_closed_and_cancels_idempotently() {
        let bin = bridge_bin();
        assert!(
            bin.is_file(),
            "bridge binary missing at {}: run sh tools/buildkit-bridge/build.sh first",
            bin.display()
        );
        let mut bridge =
            BridgeProcess::spawn(Command::new(&bin)).expect("spawn the compiled bridge");

        let capabilities = bridge.negotiate().expect("negotiation");
        assert_eq!(capabilities, vec!["protocol.v1".to_string()]);

        // Fail-closed submit: no BuildKit client is linked yet, so the
        // session must terminally fail — never fake success.
        let session = SessionId::new("build-e2e").unwrap();
        let terminal = bridge
            .submit(
                &session,
                &Digest::of(b"definition"),
                10,
                &Digest::of(b"exporter"),
            )
            .expect("drive the submitted session");
        assert_eq!(terminal, Terminal::Failed);

        // Idempotent cancel on an unknown session still acknowledges.
        let other = SessionId::new("unknown-session").unwrap();
        bridge.cancel(&other).expect("cancel acknowledges");
        bridge.cancel(&other).expect("cancel is idempotent");
    }
}
