//! Deploy: ownership modes, drift, destinations (0001 §3.7).

use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use std::path::{Path, PathBuf};

/// Run-level rollback (0001 §9, review finding E1): an apply that
/// fails mid-graph must leave NO half-applied deployment behind —
/// the generation flip was never reached, so every destination this
/// run touched is restored to the previous generation's state (or
/// removed if the previous generation didn't deploy it).
pub(crate) fn run_rollback(
    touched: &std::collections::BTreeMap<String, store::ModuleState>,
    prev: &std::collections::BTreeMap<String, store::ModuleState>,
) {
    for (name, state) in touched {
        let prev_state = prev.get(name);
        for entry in &state.entries {
            let dest = expand_home(&entry.to);
            match prev_state.and_then(|p| p.entries.iter().find(|e| e.to == entry.to)) {
                Some(prev_entry) => {
                    let source = prev_state
                        .expect("matched above")
                        .store_path
                        .join(&prev_entry.from);
                    let result = match prev_entry.mode {
                        Ownership::Owned => store::symlink_replace(&dest, &source),
                        _ => std::fs::read(&source)
                            .and_then(|bytes| store::atomic_write(&dest, &bytes)),
                    };
                    if result.is_ok() {
                        tracing::info!("restored {} (run rolled back)", entry.to);
                    }
                }
                None => {
                    if std::fs::remove_file(&dest).is_ok() {
                        tracing::info!("removed {} (run rolled back)", entry.to);
                    }
                }
            }
        }
    }
}

/// never silently overwrite.
pub(crate) fn deploy_entry(
    out: &mut Vec<store::DeployedEntry>,
    module: &str,
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
        module: module.to_string(),
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
            "{:?} on a directory ({}) — directory payloads are not supported yet; owned symlinks work today",
            entry.mode, entry.from
        )));
    }
    let hash = store::canonical_file_hash(&source)?;
    let (summary, kind) = match &entry.mode {
        Ownership::Owned => {
            // external satisfaction (0009 critique): never overwrite a
            // path that is neither ours (symlink into the store) nor
            // recorded in a previous manifest — unless --take-over.
            let recorded = prev
                .map(|m| m.entries.iter().any(|e| e.to == entry.to))
                .unwrap_or(false);
            let ours = std::fs::read_link(&dest)
                .map(|t| t.starts_with(&ctx.home))
                .unwrap_or(false);
            // foreign symlinks refuse too (review finding E4): a stow/
            // chezmoi link is exactly the foreign path this guard is
            // for — absorb it only via --take-over, never silently
            if dest.symlink_metadata().is_ok() && !ours && !recorded && !ctx.take_over {
                return Err(ExecError::Step {
                    module: module.to_string(),
                    step: "deploy".into(),
                    detail: format!(
                        "{} exists and was not deployed by gripsack — move it away or use --take-over",
                        entry.to
                    ),
                });
            }
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
                detail: format!("ownership mode {other:?} is not supported yet"),
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
