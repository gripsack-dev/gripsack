//! Deploy: ownership modes, drift, destinations (0001 §3.7).

use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use std::path::{Path, PathBuf};

/// never silently overwrite.
pub(crate) fn deploy_entry(
    out: &mut Vec<store::DeployedEntry>,
    store_path: &Path,
    entry: &Entry,
    ctx: &Ctx,
    prev: Option<&store::ModuleState>,
    version: Option<&str>,
) -> Result<(String, ReportKind), ExecError> {
    let from = match version {
        Some(v) => entry.from.replace("{version}", v),
        None => entry.from.clone(),
    };
    let source = resolve_source(store_path, &from, &ctx.repo);
    let dest = expand_home(&entry.to);
    let fail = |detail: String| ExecError::Step {
        module: entry.from.clone(),
        step: "deploy".into(),
        detail,
    };
    if !source.exists() {
        return Err(fail(format!(
            "no payload or repo file at {} (from {})",
            source.display(),
            entry.from
        )));
    }
    if source.is_dir() && entry.mode != Ownership::Owned {
        return Err(fail(format!(
            "{:?} on a directory ({}) — tree deploys land in 0.2; owned symlinks work today",
            entry.mode, entry.from
        )));
    }
    let hash = store::canonical_file_hash(&source)?;
    let (summary, kind) = match &entry.mode {
        Ownership::Owned => {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            store::symlink_replace(&dest, &source)?;
            (
                format!("linked {} → {}", from, entry.to),
                ReportKind::Installed,
            )
        }
        Ownership::TrackedCopy => {
            let prev_hash = prev
                .and_then(|m| m.entries.iter().find(|e| e.to == entry.to))
                .map(|e| e.hash.as_str());
            if dest.exists() {
                let current = store::canonical_file_hash(&dest)?;
                if current == hash {
                    (format!("{} unchanged", entry.to), ReportKind::Satisfied)
                } else if prev_hash == Some(current.as_str()) {
                    store::atomic_write(&dest, &std::fs::read(&source)?)?;
                    (
                        format!("updated {} → {}", from, entry.to),
                        ReportKind::Configured,
                    )
                } else if ctx.take_over {
                    store::atomic_write(&dest, &std::fs::read(&source)?)?;
                    (
                        format!("took over {} → {}", from, entry.to),
                        ReportKind::Configured,
                    )
                } else {
                    let note = if prev_hash.is_none() {
                        format!("{} exists (not deployed by gripsack) — kept", entry.to)
                    } else {
                        format!("{} drifted — kept", entry.to)
                    };
                    tracing::warn!("{}", note);
                    (note, ReportKind::Warned)
                }
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                store::atomic_write(&dest, &std::fs::read(&source)?)?;
                (
                    format!("copied {} → {}", from, entry.to),
                    ReportKind::Configured,
                )
            }
        }
        other => {
            return Err(ExecError::Step {
                module: entry.from.clone(),
                step: "deploy".into(),
                detail: format!("ownership mode {other:?} lands in 0.2"),
            });
        }
    };
    out.push(store::DeployedEntry {
        from: entry.from.clone(),
        to: entry.to.clone(),
        mode: entry.mode.clone(),
        hash,
    });
    Ok((summary, kind))
}

/// Entry content lives in the store payload if present, else in the
/// repo (config files travel with the env repo — 0006).
fn resolve_source(store_path: &Path, from: &str, repo: &Path) -> PathBuf {
    let in_store = store_path.join(from);
    if in_store.exists() {
        in_store
    } else {
        repo.join(from)
    }
}

pub(crate) fn expand_home(to: &str) -> PathBuf {
    if let Some(rest) = to.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(to)
}
