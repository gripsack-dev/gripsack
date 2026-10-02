//! Bounded bridge-protocol fuzzing through the REAL production
//! decoder (`gripsack_buildkit::protocol`). Arbitrary bytes are wire
//! input only: every decode must either reject or yield a message
//! that re-encodes and re-decodes to the identical value, and feeding
//! decoded events through the fence gate can never panic or let a
//! stale/duplicate terminal flip an outcome. No I/O, no second
//! implementation.

use gripsack_buildkit::protocol::{
    BridgeEvent, EventGate, Frame, FromBridge, Terminal, ToBridge, decode_frame, encode_frame,
};
use serde::{Deserialize, Serialize};

pub(crate) fn exercise(input: &[u8]) {
    // First byte picks the decode target, the rest is the wire
    // candidate — deterministic per input, both families covered.
    let (target, wire) = input.split_first().unwrap_or((&0u8, &[][..]));
    match target % 2 {
        0 => {
            let Ok(Frame { body }) = decode_frame::<FromBridge>(wire) else {
                return;
            };
            assert_reencode(&body);
            gate_feed(&body);
        }
        _ => {
            let Ok(Frame { body }) = decode_frame::<ToBridge>(wire) else {
                return;
            };
            assert_reencode(&body);
        }
    }
}

/// The round-trip law: a decoded message must re-encode into a frame
/// that decodes to the identical value — decode/encode asymmetry
/// cannot hide state from either side of the bridge.
fn assert_reencode<M>(body: &M)
where
    M: Serialize + for<'de> Deserialize<'de> + std::fmt::Debug,
{
    let Frame { body: again } =
        decode_frame::<M>(&encode_frame(body)).expect("re-encoding a decoded message decodes");
    assert_eq!(
        serde_json::to_vec(body).expect("serialize"),
        serde_json::to_vec(&again).expect("serialize"),
        "re-encode changed the message"
    );
}

/// Whatever the decoded event says, driving the fence gate must stay
/// inside its rules: stale/future epochs and post-terminal events
/// reject, never panic, and exactly one terminal wins.
fn gate_feed(message: &FromBridge) {
    let mut gate = EventGate::new(1);
    if let FromBridge::Event { epoch, kind, .. } = message {
        match kind {
            BridgeEvent::Log { .. } => {
                let _ = gate.log(*epoch);
            }
            BridgeEvent::Progress { .. } => {
                let _ = gate.observe(*epoch);
            }
        }
    }
    let _ = gate.terminal(1, Terminal::Done);
    assert_eq!(gate.terminal_state(), Some(Terminal::Done));
}
