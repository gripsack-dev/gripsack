//! Bounded execution metadata, never command arguments, environment values or
//! child output. Receipts describe enforcement; they do not attest native code
//! or promise confinement of descendants that leave the process group.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByteBinding {
    /// The launched image is addressed through the retained executable handle.
    ExecutableHandle,
    /// Hashed bytes copied privately, but execution still reopens a pathname.
    PrivateCopyPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    ProcessGroup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessDisposition {
    Exited,
    Deadline,
    InputLimit,
    LineLimit,
    StdoutLimit,
    StderrLimit,
    Io,
    CleanupFailure,
    SpawnFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessReceipt {
    pub executable_sha256: super::Sha256Digest,
    pub script_sha256: Option<super::Sha256Digest>,
    pub byte_binding: ByteBinding,
    pub enforcement: Enforcement,
    pub environment_keys: Vec<String>,
    pub deadline_millis: u64,
    pub stdout_limit: u64,
    pub stderr_limit: u64,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub disposition: ProcessDisposition,
    pub error: Option<super::NativeIoError>,
    pub cleanup_cause: Option<ProcessDisposition>,
}
