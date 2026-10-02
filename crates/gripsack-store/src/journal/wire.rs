//! Strict journal wire admission. v1 retains its one-mutation interpretation;
//! v2 also records the immediate pre-mutation state while preserving the run's
//! original prior. Older binaries reject v2 rather than misrecovering it.
use super::Prior;

pub(super) const ENTRY_WIRE_VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Entry {
    v: u8,
    pub dest: String,
    /// Original state before this run first touched the destination.
    pub prior: PriorSerde,
    /// Absent only in historical v1, never defaulted for v2 input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) before: Option<PriorSerde>,
    pub after: IntendedSerde,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IntendedSerde {
    Removed,
    File { identity: crate::hash::FileIdentity },
    Link { target: String },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PriorSerde {
    Absent,
    File {
        hash: crate::prior::PriorBlobId,
        mode: crate::prior::FileMode,
    },
    Symlink {
        target: String,
    },
}

impl From<&Prior> for PriorSerde {
    fn from(prior: &Prior) -> Self {
        match prior {
            Prior::Absent => Self::Absent,
            Prior::File { hash, mode } => Self::File {
                hash: hash.clone(),
                mode: *mode,
            },
            Prior::Symlink { target } => Self::Symlink {
                target: target.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireRejection {
    Legacy,
    UnsupportedVersion(u8),
    Malformed(String),
}

impl std::fmt::Display for WireRejection {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Legacy => output.write_str("pre-0.40 untagged entry — its intent cannot be proven from the bare string, so it is retained for inspection instead of guessed"),
            Self::UnsupportedVersion(version) => write!(output, "journal entry wire version {version} is newer than this grip understands"),
            Self::Malformed(why) => write!(output, "malformed journal entry: {why}"),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WireV1 {
    v: u8,
    dest: String,
    prior: PriorSerde,
    after: IntendedSerde,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WireV2 {
    v: u8,
    dest: String,
    prior: PriorSerde,
    before: PriorSerde,
    after: IntendedSerde,
}

impl Entry {
    pub fn new(dest: String, prior: PriorSerde, after: IntendedSerde) -> Self {
        Self {
            v: ENTRY_WIRE_VERSION,
            dest,
            before: Some(prior.clone()),
            prior,
            after,
        }
    }

    pub(super) fn advance(self, before: PriorSerde, after: IntendedSerde) -> Self {
        Self {
            v: ENTRY_WIRE_VERSION,
            before: Some(before),
            after,
            ..self
        }
    }

    pub fn from_wire(bytes: &[u8]) -> Result<Self, WireRejection> {
        let malformed = |error: serde_json::Error| WireRejection::Malformed(error.to_string());
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(malformed)?;
        let object = value
            .as_object()
            .ok_or_else(|| WireRejection::Malformed("not a JSON object".into()))?;
        if object
            .get("after")
            .is_some_and(serde_json::Value::is_string)
        {
            return Err(WireRejection::Legacy);
        }
        let version = object
            .get("v")
            .and_then(serde_json::Value::as_u64)
            .and_then(|version| u8::try_from(version).ok())
            .ok_or_else(|| WireRejection::Malformed("missing or invalid wire version".into()))?;
        // Decode ORIGINAL bytes, not the Value above: duplicate fields must
        // still be rejected by serde instead of disappearing in a map.
        let entry = match version {
            1 => {
                let wire: WireV1 = serde_json::from_slice(bytes).map_err(malformed)?;
                Self {
                    v: wire.v,
                    dest: wire.dest,
                    prior: wire.prior,
                    before: None,
                    after: wire.after,
                }
            }
            ENTRY_WIRE_VERSION => {
                let wire: WireV2 = serde_json::from_slice(bytes).map_err(malformed)?;
                Self {
                    v: wire.v,
                    dest: wire.dest,
                    prior: wire.prior,
                    before: Some(wire.before),
                    after: wire.after,
                }
            }
            other => return Err(WireRejection::UnsupportedVersion(other)),
        };
        if let IntendedSerde::File { identity } = &entry.after
            && (identity.as_str().len() != 64
                || !identity
                    .as_str()
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(WireRejection::Malformed(
                "invalid intended file identity".into(),
            ));
        }
        Ok(entry)
    }
}
