//! Historical path approvals remain inspectable, never source authority.
use super::policy::{EvaluationPolicy, EvaluationPolicyDigest};
use crate::source_bundle::SourceBundleDigest;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Component, Path},
};

pub(super) const TRUST_VERSION: u32 = 2;
pub(super) const MAX_TRUST_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_APPROVALS: usize = 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitProvenance {
    pub remote: Option<String>,
    pub commit: Option<String>,
}

impl GitProvenance {
    /// Admit informational native-query results without retaining credentials.
    pub fn from_git(remote: Option<&str>, commit: Option<String>) -> Self {
        Self {
            remote: remote.and_then(super::audit::redact_remote),
            commit: commit.filter(|value| {
                matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedSource {
    pub repository: String,
    pub bundle: SourceBundleDigest,
    pub policy_digest: EvaluationPolicyDigest,
    pub policy: EvaluationPolicy,
    pub provenance: GitProvenance,
    pub approved_at: String,
}

impl ApprovedSource {
    pub(super) fn validate(&self) -> io::Result<()> {
        validate_repository(&self.repository)?;
        self.policy.validate()?;
        if self.bundle != self.policy.frontend.bundle || self.policy.digest()? != self.policy_digest
        {
            return Err(invalid(
                "stored source approval has inconsistent source/policy identities",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyRepo {
    pub path: String,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    pub trusted_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustListing {
    pub version: u32,
    pub approved: Vec<ApprovedSource>,
    pub legacy: Vec<LegacyRepo>,
}

impl Default for TrustListing {
    fn default() -> Self {
        Self {
            version: TRUST_VERSION,
            approved: Vec::new(),
            legacy: Vec::new(),
        }
    }
}

impl TrustListing {
    pub(super) fn validate(&self) -> io::Result<()> {
        if self.version != TRUST_VERSION {
            return Err(invalid("unsupported trust document version"));
        }
        if self
            .approved
            .len()
            .checked_add(self.legacy.len())
            .is_none_or(|count| count > MAX_APPROVALS)
        {
            return Err(invalid("trust document exceeds its approval budget"));
        }
        let mut paths = std::collections::BTreeSet::new();
        for entry in &self.approved {
            entry.validate()?;
            if !paths.insert(&entry.repository) {
                return Err(invalid("duplicate source approval repository"));
            }
        }
        for entry in &self.legacy {
            validate_repository(&entry.path)?;
            if !paths.insert(&entry.path) {
                return Err(invalid("duplicate legacy/source approval repository"));
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyTrustFile {
    #[serde(default)]
    pub repos: Vec<LegacyRepo>,
}

pub(super) fn validate_repository(repository: &str) -> io::Result<()> {
    let path = Path::new(repository);
    if !path.is_absolute()
        || repository.contains('\0')
        || repository.contains("//")
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid(
            "source approval repository is not an absolute canonical path",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
