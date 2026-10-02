//! Approval of captured source and evaluator/native policy, not a live path.
//! Historical path-only records remain visible but cannot authorize evaluation.
mod audit;
pub mod evaluation;
mod policy;
mod storage;
#[cfg(test)]
mod tests;
mod wire;

use crate::source_bundle::{SourceBundle, SourceBundleDigest, SourceEntry};
pub use policy::{
    EvaluationPolicy, EvaluationPolicyDigest, EvaluatorBudget, EvaluatorProfile, FrontendIdentity,
    NativeDeclarations, ProbeCapability,
};
use std::{io, path::Path};
pub use wire::{ApprovedSource, GitProvenance, LegacyRepo, TrustListing};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApprovalStatus {
    Unapproved,
    Legacy,
    Approved,
    Changed {
        source_changed: bool,
        policy_changed: bool,
    },
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryChangeKind {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, serde::Serialize)]
pub struct InventoryChange {
    pub path: String,
    pub change: InventoryChangeKind,
}

#[derive(Debug, serde::Serialize)]
pub struct ApprovalInspection {
    pub status: ApprovalStatus,
    pub previous_bundle: Option<SourceBundleDigest>,
    pub changes: Vec<InventoryChange>,
}

pub fn list(home: &Path) -> io::Result<TrustListing> {
    storage::load(home)
}

fn repository<'a>(bundle: &'a SourceBundle, policy: &EvaluationPolicy) -> io::Result<&'a str> {
    let path = bundle
        .repository_identity()
        .to_str()
        .ok_or_else(|| invalid("repository identity is not UTF-8"))?;
    wire::validate_repository(path)?;
    policy.validate()?;
    if policy.frontend.bundle != bundle.digest() || policy.read_roots != bundle.inventory().roots()
    {
        return Err(invalid(
            "evaluation policy is not bound to this captured source bundle",
        ));
    }
    Ok(path)
}

fn classify(
    listing: &TrustListing,
    repository: &str,
    bundle: SourceBundleDigest,
    policy: EvaluationPolicyDigest,
) -> ApprovalStatus {
    if let Some(previous) = listing
        .approved
        .iter()
        .find(|entry| entry.repository == repository)
    {
        let source_changed = previous.bundle != bundle;
        let policy_changed = previous.policy_digest != policy;
        if source_changed || policy_changed {
            ApprovalStatus::Changed {
                source_changed,
                policy_changed,
            }
        } else {
            ApprovalStatus::Approved
        }
    } else if listing.legacy.iter().any(|entry| entry.path == repository) {
        ApprovalStatus::Legacy
    } else {
        ApprovalStatus::Unapproved
    }
}

/// Ordinary gate lookup avoids constructing an inventory diff. The retained
/// inventory must still exactly match the freshly captured admitted bytes.
pub fn status(
    home: &Path,
    bundle: &SourceBundle,
    policy: &EvaluationPolicy,
) -> io::Result<ApprovalStatus> {
    let repository = repository(bundle, policy)?;
    let listing = storage::load(home)?;
    let status = classify(&listing, repository, bundle.digest(), policy.digest()?);
    if status == ApprovalStatus::Approved {
        storage::admit_inventory(home, bundle)?;
    }
    Ok(status)
}

pub fn inspect(
    home: &Path,
    bundle: &SourceBundle,
    policy: &EvaluationPolicy,
) -> io::Result<ApprovalInspection> {
    let repository = repository(bundle, policy)?;
    let listing = storage::load(home)?;
    let status = classify(&listing, repository, bundle.digest(), policy.digest()?);
    let previous = listing
        .approved
        .iter()
        .find(|entry| entry.repository == repository);
    let changes = match previous {
        Some(previous) if previous.bundle == bundle.digest() => {
            storage::admit_inventory(home, bundle)?;
            Vec::new()
        }
        Some(previous) => {
            let inventory = storage::inventory(home, previous.bundle)?;
            difference(inventory.entries(), bundle.inventory().entries())
        }
        None => bundle
            .inventory()
            .entries()
            .iter()
            .map(|entry| InventoryChange {
                path: entry.path.clone(),
                change: InventoryChangeKind::Added,
            })
            .collect(),
    };
    Ok(ApprovalInspection {
        status,
        previous_bundle: previous.map(|entry| entry.bundle),
        changes,
    })
}

/// Explicit approval compares both values shown by inspection. A runtime or
/// native-policy change between inspect/add cannot piggyback on unchanged source.
pub fn approve(
    home: &Path,
    bundle: &SourceBundle,
    policy: &EvaluationPolicy,
    expected_bundle: SourceBundleDigest,
    expected_policy: EvaluationPolicyDigest,
    provenance: &GitProvenance,
) -> io::Result<ApprovedSource> {
    let repository = repository(bundle, policy)?;
    let policy_digest = policy.digest()?;
    if expected_bundle != bundle.digest() || expected_policy != policy_digest {
        return Err(invalid(
            "captured source or policy differs from the explicit approval; inspect again",
        ));
    }
    let _lock = storage::lock(home)?;
    let mut listing = storage::load(home)?;
    storage::save_inventory(home, bundle)?;
    listing
        .approved
        .retain(|entry| entry.repository != repository);
    listing.legacy.retain(|entry| entry.path != repository);
    listing.approved.push(ApprovedSource {
        repository: repository.to_owned(),
        bundle: bundle.digest(),
        policy_digest,
        policy: policy.clone(),
        provenance: GitProvenance {
            remote: provenance.remote.as_deref().and_then(audit::redact_remote),
            commit: provenance.commit.clone(),
        },
        approved_at: audit::now_rfc3339(),
    });
    storage::save(home, &listing)?;
    Ok(listing
        .approved
        .pop()
        .expect("the saved approval was just appended"))
}

/// Revocation is by repository identity, including legacy records. A missing
/// repository may still be removed by its recorded absolute path; this fallback
/// cannot create or match evaluation authority.
pub fn remove(home: &Path, repo: &Path) -> io::Result<bool> {
    let repository = match repo.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => std::path::absolute(repo)?,
        Err(error) => return Err(error),
    };
    let _lock = storage::lock(home)?;
    let mut listing = storage::load(home)?;
    let before = listing.approved.len() + listing.legacy.len();
    listing
        .approved
        .retain(|entry| Path::new(&entry.repository) != repository);
    listing
        .legacy
        .retain(|entry| Path::new(&entry.path) != repository);
    let removed = before != listing.approved.len() + listing.legacy.len();
    if removed {
        storage::save(home, &listing)?;
    }
    Ok(removed)
}

fn difference(previous: &[SourceEntry], current: &[SourceEntry]) -> Vec<InventoryChange> {
    let mut changes = Vec::new();
    let (mut before, mut after) = (previous.iter().peekable(), current.iter().peekable());
    loop {
        let (path, change) = match (before.peek(), after.peek()) {
            (Some(&left), Some(&right)) if left.path == right.path => {
                let changed = left.object != right.object;
                let path = &right.path;
                before.next();
                after.next();
                if changed {
                    (path, InventoryChangeKind::Changed)
                } else {
                    continue;
                }
            }
            (Some(&left), Some(&right)) if left.path < right.path => {
                let path = &left.path;
                before.next();
                (path, InventoryChangeKind::Removed)
            }
            (Some(_), Some(&right)) | (None, Some(&right)) => {
                let path = &right.path;
                after.next();
                (path, InventoryChangeKind::Added)
            }
            (Some(&left), None) => {
                let path = &left.path;
                before.next();
                (path, InventoryChangeKind::Removed)
            }
            (None, None) => break,
        };
        changes.push(InventoryChange {
            path: path.clone(),
            change,
        });
    }
    changes
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
