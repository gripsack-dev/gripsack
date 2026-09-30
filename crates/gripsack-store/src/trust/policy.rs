//! Versioned identities for the actual evaluator and declared native policy.
//! Native configuration values are hashed, never retained in approval output.
use crate::source_bundle::{SourceBundle, SourceBundleDigest, SourceRootKind};
use gripsack_process::{Limits, ProgramIdentity, Sha256Digest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Write},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvaluationPolicyDigest(Sha256Digest);
impl EvaluationPolicyDigest {
    pub fn parse(value: &str) -> io::Result<Self> {
        Sha256Digest::parse(value).map(Self)
    }
}
impl fmt::Display for EvaluationPolicyDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorProfile {
    /// Captured sources + one core-generated round file; no env/network/run/
    /// FFI/sys grants, no remote/package fetching or ambient config discovery.
    CapturedReadOnlyV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCapability {
    ExecutableLookup,
    FileExistence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeDeclarations {
    /// Evaluation may declare native fetch/build/verify/activation actions;
    /// their execution still requires the selected command's existing gates.
    ExplicitCommandsV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontendIdentity {
    pub bundle: SourceBundleDigest,
    pub implementation: SourceRootKind,
    pub driver: SourceRootKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorBudget {
    pub operation_millis: u64,
    pub input_bytes: usize,
    pub input_document_bytes: usize,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub retained_stderr_bytes: usize,
    pub rounds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationPolicy {
    version: u32,
    pub profile: EvaluatorProfile,
    pub read_roots: Vec<SourceRootKind>,
    pub frontend: FrontendIdentity,
    pub runtime: ProgramIdentity,
    pub probes: Vec<ProbeCapability>,
    pub native_declarations: NativeDeclarations,
    pub native_configuration_sha256: Sha256Digest,
    pub budget: EvaluatorBudget,
}

impl EvaluationPolicy {
    pub fn capture(
        bundle: &SourceBundle,
        runtime: ProgramIdentity,
        native_configuration: &impl Serialize,
        limits: Limits,
        rounds: u32,
        input_document_bytes: usize,
    ) -> io::Result<Self> {
        let policy = Self {
            version: 1,
            profile: EvaluatorProfile::CapturedReadOnlyV1,
            read_roots: bundle.inventory().roots().to_vec(),
            frontend: FrontendIdentity {
                bundle: bundle.digest(),
                implementation: if bundle.pinned_frontend().is_some() {
                    SourceRootKind::PinnedFrontend
                } else {
                    SourceRootKind::Frontend
                },
                driver: SourceRootKind::Frontend,
            },
            runtime,
            probes: vec![
                ProbeCapability::ExecutableLookup,
                ProbeCapability::FileExistence,
            ],
            native_declarations: NativeDeclarations::ExplicitCommandsV1,
            native_configuration_sha256: digest_json(native_configuration)?,
            budget: EvaluatorBudget {
                operation_millis: u64::try_from(limits.timeout.as_millis())
                    .map_err(|_| invalid("evaluator duration exceeds its receipt domain"))?,
                input_bytes: limits.input_bytes.bytes(),
                input_document_bytes,
                stdout_bytes: limits.stdout_bytes.bytes(),
                stderr_bytes: limits.stderr_bytes.bytes(),
                retained_stderr_bytes: limits.retained_stderr_bytes.bytes(),
                rounds,
            },
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn digest(&self) -> io::Result<EvaluationPolicyDigest> {
        self.validate()?;
        digest_json(self).map(EvaluationPolicyDigest)
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        if self.version != 1 || self.frontend.driver != SourceRootKind::Frontend {
            return Err(invalid("unsupported evaluation policy version or driver"));
        }
        let pinned = match self.read_roots.as_slice() {
            [SourceRootKind::Repository, SourceRootKind::Frontend] => false,
            [
                SourceRootKind::Repository,
                SourceRootKind::Frontend,
                SourceRootKind::PinnedFrontend,
            ] => true,
            _ => return Err(invalid("invalid evaluation read-root policy")),
        };
        let implementation = if pinned {
            SourceRootKind::PinnedFrontend
        } else {
            SourceRootKind::Frontend
        };
        if self.frontend.implementation != implementation
            || self.probes
                != [
                    ProbeCapability::ExecutableLookup,
                    ProbeCapability::FileExistence,
                ]
            || self.budget.rounds == 0
        {
            return Err(invalid(
                "evaluation policy does not describe its admitted frontend/probes",
            ));
        }
        Ok(())
    }
}

pub(super) fn digest_json(value: &impl Serialize) -> io::Result<Sha256Digest> {
    struct HashWriter(Sha256);
    impl Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = HashWriter(Sha256::new());
    serde_json::to_writer(&mut output, value).map_err(io::Error::other)?;
    Ok(Sha256Digest::from_bytes(output.0.finalize().into()))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
