//! Private evaluation history binds approved source/policy and each immutable
//! input envelope to actual process outcomes. It contains no source/output bytes.
use super::{EvaluationPolicy, EvaluationPolicyDigest, GitProvenance, audit};
use crate::source_bundle::{SourceBundle, SourceBundleDigest};
use gripsack_fs::Dir;
use gripsack_process::{ProcessDisposition, ProcessReceipt, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::{self, Read},
    path::Path,
};

const RECEIPT_VERSION: u32 = 1;
const RECEIPT_BYTES: usize = 128 * 1024;
const DIRECTORY: &str = "evaluations";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvaluationId(Sha256Digest);
impl EvaluationId {
    pub fn parse(value: &str) -> io::Result<Self> {
        Sha256Digest::parse(value).map(Self)
    }
    fn fresh() -> io::Result<Self> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(io::Error::other)?;
        Ok(Self(Sha256Digest::from_bytes(bytes)))
    }
}
impl fmt::Display for EvaluationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Frontend outcome only. A completed stable envelope is not evidence that
/// downstream IR validation, native builds or deployment subsequently succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationOutcome {
    Started,
    Rejected,
    Failed,
    Completed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationRound {
    pub number: std::num::NonZeroU32,
    pub input_sha256: Sha256Digest,
    pub process: Option<ProcessReceipt>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationReceipt {
    pub version: u32,
    pub id: EvaluationId,
    pub repository: String,
    pub source: SourceBundleDigest,
    pub policy_digest: EvaluationPolicyDigest,
    pub policy: EvaluationPolicy,
    pub provenance: GitProvenance,
    pub created_at: String,
    pub outcome: EvaluationOutcome,
    pub rounds: Vec<EvaluationRound>,
}

impl EvaluationReceipt {
    fn validate(&self) -> io::Result<()> {
        if self.version != RECEIPT_VERSION {
            return Err(invalid("unsupported evaluation receipt version"));
        }
        super::wire::validate_repository(&self.repository)?;
        if self.source != self.policy.frontend.bundle || self.policy.digest()? != self.policy_digest
        {
            return Err(invalid(
                "evaluation receipt source/policy identities disagree",
            ));
        }
        if self.rounds.len() > self.policy.budget.rounds as usize {
            return Err(invalid("evaluation receipt exceeds its round budget"));
        }
        for (index, round) in self.rounds.iter().enumerate() {
            if round.number.get() as usize != index + 1
                || (index + 1 < self.rounds.len() && round.process.is_none())
            {
                return Err(invalid(
                    "evaluation receipt has incomplete or unordered rounds",
                ));
            }
            if let Some(process) = &round.process
                && (process.executable_sha256 != self.policy.runtime.executable_sha256
                    || process.script_sha256 != self.policy.runtime.script_sha256
                    || process.byte_binding != self.policy.runtime.byte_binding)
            {
                return Err(invalid(
                    "evaluation process does not match the approved runtime",
                ));
            }
        }
        if self.outcome == EvaluationOutcome::Rejected && !self.rounds.is_empty() {
            return Err(invalid(
                "rejected evaluation contains executed input rounds",
            ));
        }
        if self.outcome == EvaluationOutcome::Completed
            && (self.rounds.is_empty()
                || self.rounds.iter().any(|round| {
                    !round.process.as_ref().is_some_and(|process| {
                        process.disposition == ProcessDisposition::Exited
                            && process.exit_code == Some(0)
                            && process.signal.is_none()
                    })
                }))
        {
            return Err(invalid(
                "completed evaluation lacks successful process evidence",
            ));
        }
        Ok(())
    }
}

pub struct EvaluationSession {
    directory: Dir,
    name: String,
    receipt: EvaluationReceipt,
}

impl EvaluationSession {
    pub fn begin(
        home: &Path,
        bundle: &SourceBundle,
        policy: &EvaluationPolicy,
        provenance: &GitProvenance,
    ) -> io::Result<Self> {
        let repository = super::repository(bundle, policy)?;
        let home = gripsack_fs::open_or_create(home)?;
        let directory = crate::private_state::ensure_directory(&home, Path::new(DIRECTORY))?;
        let id = EvaluationId::fresh()?;
        let name = format!("{id}.json");
        match directory.symlink_metadata(&name) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "evaluation identity already exists",
                ));
            }
        }
        let session = Self {
            directory,
            name,
            receipt: EvaluationReceipt {
                version: RECEIPT_VERSION,
                id,
                repository: repository.to_owned(),
                source: bundle.digest(),
                policy_digest: policy.digest()?,
                policy: policy.clone(),
                provenance: GitProvenance {
                    remote: provenance.remote.as_deref().and_then(audit::redact_remote),
                    commit: provenance.commit.clone(),
                },
                created_at: audit::now_rfc3339(),
                outcome: EvaluationOutcome::Started,
                rounds: Vec::new(),
            },
        };
        session.save()?;
        Ok(session)
    }

    pub fn id(&self) -> EvaluationId {
        self.receipt.id
    }

    pub fn record_input(&mut self, input_sha256: Sha256Digest) -> io::Result<()> {
        if self.receipt.outcome != EvaluationOutcome::Started
            || self
                .receipt
                .rounds
                .last()
                .is_some_and(|round| round.process.is_none())
        {
            return Err(invalid(
                "evaluation input does not follow completed prior work",
            ));
        }
        let number = u32::try_from(self.receipt.rounds.len())
            .ok()
            .and_then(|value| value.checked_add(1))
            .and_then(std::num::NonZeroU32::new)
            .ok_or_else(|| invalid("evaluation round count overflow"))?;
        if number.get() > self.receipt.policy.budget.rounds {
            return Err(invalid("evaluation round budget exhausted"));
        }
        self.receipt.rounds.push(EvaluationRound {
            number,
            input_sha256,
            process: None,
        });
        self.save()
    }

    pub fn record_process(&mut self, process: ProcessReceipt) -> io::Result<()> {
        let round = self
            .receipt
            .rounds
            .last_mut()
            .ok_or_else(|| invalid("evaluation process has no input envelope"))?;
        if round.process.is_some() || self.receipt.outcome != EvaluationOutcome::Started {
            return Err(invalid("evaluation process outcome is already settled"));
        }
        round.process = Some(process);
        self.save()
    }

    pub fn finish(&mut self, outcome: EvaluationOutcome) -> io::Result<()> {
        if outcome == EvaluationOutcome::Started
            || self.receipt.outcome != EvaluationOutcome::Started
        {
            return Err(invalid("evaluation outcome is already settled"));
        }
        self.receipt.outcome = outcome;
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        self.receipt.validate()?;
        let bytes = serde_json::to_vec(&self.receipt).map_err(io::Error::other)?;
        if bytes.len() > RECEIPT_BYTES {
            return Err(invalid("evaluation receipt exceeds its byte budget"));
        }
        gripsack_fs::atomic_write_with_mode(&self.directory, Path::new(&self.name), &bytes, 0o600)
    }
}

pub fn read(home: &Path, id: EvaluationId) -> io::Result<EvaluationReceipt> {
    let home = gripsack_fs::open(home)?;
    let directory = gripsack_fs::open_dir_nofollow(&home, Path::new(DIRECTORY))?;
    let name = format!("{id}.json");
    let mut file = gripsack_fs::open_file_nofollow(&directory, Path::new(&name))?;
    crate::private_state::restrict_file(&file, Path::new(&name))?;
    let length = file.metadata()?.len();
    if length > RECEIPT_BYTES as u64 {
        return Err(invalid("evaluation receipt exceeds its byte budget"));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    (&mut file)
        .take(RECEIPT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > RECEIPT_BYTES {
        return Err(invalid("evaluation receipt exceeds its byte budget"));
    }
    let mut receipt: EvaluationReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    receipt.validate()?;
    if receipt.id != id {
        return Err(invalid(
            "evaluation receipt identity differs from its filename",
        ));
    }
    receipt.provenance.remote = receipt
        .provenance
        .remote
        .as_deref()
        .and_then(audit::redact_remote);
    Ok(receipt)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
