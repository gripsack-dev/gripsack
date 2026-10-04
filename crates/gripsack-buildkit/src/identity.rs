//! Distinct byte and operation identities at the untrusted bridge boundary.
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Serialize};
use std::{fmt, io};

macro_rules! digest_domain {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Sha256Digest);
        impl $name {
            pub fn of(bytes: &[u8]) -> Self {
                Self(Sha256Digest::of(bytes))
            }
            pub fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(Sha256Digest::from_bytes(bytes))
            }
            pub fn parse(text: &str) -> io::Result<Self> {
                Sha256Digest::parse(text).map(Self)
            }
            pub fn bytes(&self) -> &[u8; 32] {
                self.0.as_bytes()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(out)
            }
        }
    };
}
digest_domain!(DefinitionDigest);
digest_domain!(ExporterDigest);
digest_domain!(SnapshotDigest);
digest_domain!(WorkerHomeId);
digest_domain!(WorkerOwnerId);
digest_domain!(WorkerInstanceId);

/// The upstream digest spelling includes the algorithm, unlike host receipts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LlbVertexDigest(String);
impl LlbVertexDigest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(format!("sha256:{}", Sha256Digest::of(bytes)))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for LlbVertexDigest {
    type Error = io::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let digest = value.strip_prefix("sha256:").ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "LLB vertex must use SHA-256")
        })?;
        Sha256Digest::parse(digest)?;
        Ok(Self(value))
    }
}
impl From<LlbVertexDigest> for String {
    fn from(value: LlbVertexDigest) -> Self {
        value.0
    }
}
impl fmt::Display for LlbVertexDigest {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionId(String);
impl SessionId {
    pub fn new(value: impl Into<String>) -> io::Result<Self> {
        Self::try_from(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for SessionId {
    type Error = io::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !safe_atom(&value) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "session must be 1..=64 ASCII letters, digits, '-' or '_'",
            ));
        }
        Ok(Self(value))
    }
}
impl From<SessionId> for String {
    fn from(value: SessionId) -> Self {
        value.0
    }
}

macro_rules! counter_domain {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(std::num::NonZeroU64);
        impl $name {
            pub fn new(value: u64) -> io::Result<Self> {
                std::num::NonZeroU64::new(value).map(Self).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        concat!(stringify!($name), " must be nonzero"),
                    )
                })
            }
            pub fn get(self) -> u64 {
                self.0.get()
            }
        }
    };
}
counter_domain!(AttemptId);
counter_domain!(FenceEpoch);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptIdentity {
    pub session: SessionId,
    pub attempt: AttemptId,
    pub epoch: FenceEpoch,
}

pub(crate) fn safe_atom(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

impl std::borrow::Borrow<str> for LlbVertexDigest {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
