//! Versioned two-phase bridge messages. Framing is bounded before allocation;
//! execution requests are constructed only from independently checked LLB.
mod failure;
mod framing;
pub use framing::{Frame, FrameDecoder, decode_frame, encode_frame};

use crate::identity::{
    AttemptId, AttemptIdentity, DefinitionDigest, ExporterDigest, FenceEpoch, LlbVertexDigest,
    SessionId, SnapshotDigest, WorkerInstanceId,
};
use crate::plan::{BuildPlan, ExporterPlan, NodeIndex, Platform};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 3;
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_LOG_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_LOG_EVENTS: usize = 4096;
pub const MAX_EVENT_COUNT: usize = 8192;
pub const EXPECTED_DAEMON_VERSION: &str = "v0.33.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LowerRequest<P = BuildPlan> {
    pub protocol_version: u32,
    pub session: SessionId,
    pub attempt: AttemptId,
    pub epoch: FenceEpoch,
    pub plan: P,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding {
    pub name: String,
    pub digest: SnapshotDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerBinding {
    pub instance: WorkerInstanceId,
    pub epoch: FenceEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteRequest {
    pub protocol_version: u32,
    pub session: SessionId,
    pub attempt: AttemptId,
    pub epoch: FenceEpoch,
    pub worker: WorkerBinding,
    #[serde(with = "bytes")]
    pub definition: Vec<u8>,
    pub definition_digest: DefinitionDigest,
    pub exporter: ExporterPlan,
    pub exporter_digest: ExporterDigest,
    pub sources: Vec<SourceBinding>,
    pub platform: Platform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeWitness {
    pub node: NodeIndex,
    pub vertex: LlbVertexDigest,
    pub output: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lowered {
    #[serde(with = "bytes")]
    pub definition: Vec<u8>,
    pub witness: Vec<NodeWitness>,
    pub exporter: ExporterPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    Rejected,
    WorkerDied,
    ExportFailed,
    InvalidOutput,
    Internal,
}

/// Externally tagged variants deliberately repeat the identity fields: serde's
/// flatten + deny_unknown_fields combination cannot provide strict admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum FromBridge {
    Prepared {
        session: SessionId,
        attempt: AttemptId,
        epoch: FenceEpoch,
        lowered: Lowered,
    },
    Accepted {
        session: SessionId,
        attempt: AttemptId,
        epoch: FenceEpoch,
        worker: WorkerBinding,
        daemon_version: String,
        platform: Platform,
    },
    Event {
        session: SessionId,
        attempt: AttemptId,
        epoch: FenceEpoch,
        worker: WorkerBinding,
        vertex: String,
        #[serde(with = "log_bytes")]
        chunk: Vec<u8>,
        truncated: bool,
    },
    Exported {
        session: SessionId,
        attempt: AttemptId,
        epoch: FenceEpoch,
        worker: WorkerBinding,
    },
    Done {
        session: SessionId,
        attempt: AttemptId,
        epoch: FenceEpoch,
        worker: WorkerBinding,
    },
    Failed {
        session: SessionId,
        attempt: AttemptId,
        epoch: FenceEpoch,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worker: Option<WorkerBinding>,
        code: FailureCode,
        message: String,
        #[serde(deserialize_with = "failure::vertices")]
        vertices: Vec<LlbVertexDigest>,
    },
    Cancelled {
        session: SessionId,
        attempt: AttemptId,
        epoch: FenceEpoch,
        worker: WorkerBinding,
    },
}
impl FromBridge {
    pub fn matches(&self, expected: &AttemptIdentity) -> bool {
        let (session, attempt, epoch) = match self {
            Self::Prepared {
                session,
                attempt,
                epoch,
                ..
            }
            | Self::Accepted {
                session,
                attempt,
                epoch,
                ..
            }
            | Self::Event {
                session,
                attempt,
                epoch,
                ..
            }
            | Self::Exported {
                session,
                attempt,
                epoch,
                ..
            }
            | Self::Done {
                session,
                attempt,
                epoch,
                ..
            }
            | Self::Failed {
                session,
                attempt,
                epoch,
                ..
            }
            | Self::Cancelled {
                session,
                attempt,
                epoch,
                ..
            } => (session, attempt, epoch),
        };
        session == &expected.session && attempt == &expected.attempt && epoch == &expected.epoch
    }
    pub fn worker_binding(&self) -> Option<WorkerBinding> {
        match self {
            Self::Prepared { .. } => None,
            Self::Accepted { worker, .. }
            | Self::Event { worker, .. }
            | Self::Exported { worker, .. }
            | Self::Done { worker, .. }
            | Self::Cancelled { worker, .. } => Some(*worker),
            Self::Failed { worker, .. } => *worker,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("bridge frame declares {0} bytes, exceeding {MAX_FRAME_BYTES}")]
    OversizedFrame(u64),
    #[error("bridge ended with a truncated frame")]
    TruncatedFrame,
    #[error("unexpected bytes after one bridge frame")]
    TrailingBytes,
    #[error("invalid bridge JSON: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("bridge request cannot be encoded: {0}")]
    Encode(#[from] std::io::Error),
    #[error("bridge event is bound to another session, attempt, worker or fence")]
    Identity,
    #[error("bridge event violates the admitted session transition")]
    Transition,
    #[error("bridge exceeded the bounded event budget")]
    EventBudget,
    #[error("bridge capability/version/platform does not match the admitted worker")]
    Capability,
}

/// Base64 keeps binary LLB and bounded log chunks inside structured framing;
/// package/export payload bytes never travel through this protocol.
pub(crate) mod bytes {
    use base64::{Engine, display::Base64Display, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&Base64Display::new(value, &STANDARD))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(decoder: D) -> Result<Vec<u8>, D::Error> {
        bounded_decode(decoder, crate::plan::MAX_DEFINITION_BYTES)
    }
    pub(super) fn bounded_decode<'de, D: Deserializer<'de>>(
        decoder: D,
        cap: usize,
    ) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(decoder)?;
        if encoded.len() > cap.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom(
                "base64 payload exceeds its byte bound",
            ));
        }
        let value = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
        if value.len() > cap {
            return Err(serde::de::Error::custom(
                "decoded payload exceeds its byte bound",
            ));
        }
        Ok(value)
    }
}
mod log_bytes {
    pub use super::bytes::serialize;
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Vec<u8>, D::Error> {
        super::bytes::bounded_decode(decoder, super::MAX_LOG_CHUNK_BYTES)
    }
}
