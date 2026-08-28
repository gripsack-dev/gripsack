//! Deploy: ownership modes, drift, destinations (0001 §3.7).

use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use std::path::{Path, PathBuf};

/// Restore one destination to what a generation's manifest recorded —
/// the ONE deploy-restore path, shared by run-level rollback and
/// `grip rollback` (0001 §3.5): every mode gets its correct semantics,
/// never a naive byte copy (template re-renders with the recorded
/// vars; merge re-upserts only the block into the foreign file).
pub fn restore_entry(
    dest: &Path,
    entry: &store::DeployedEntry,
    store_path: &Path,
    module: &str,
) -> std::io::Result<()> {
    let source = store_path.join(&entry.from);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match entry.mode {
        Ownership::Owned => store::symlink_replace(dest, &source),
        Ownership::Merge => {
            let payload = std::fs::read_to_string(&source).unwrap_or_default();
            let existing = std::fs::read_to_string(dest).unwrap_or_default();
            match crate::render::upsert_block(&existing, module, dest, None, &payload) {
                Ok(new) => store::atomic_write(dest, new.as_bytes()),
                Err(_) => Ok(()), // malformed markers: leave the foreign file alone
            }
        }
        Ownership::Template => std::fs::read(&source).and_then(|raw| {
            crate::render::render_template(&raw, &entry.vars, &entry.from)
                .map_err(std::io::Error::other)
                .and_then(|bytes| store::atomic_write(dest, &bytes))
        }),
        Ownership::TrackedCopy => {
            std::fs::read(&source).and_then(|bytes| store::atomic_write(dest, &bytes))
        }
    }
}

/// Remove a destination we deployed, with drift guards (0001 §3.5):
/// never delete user edits. Returns true if anything was removed.
/// Merge entries remove only our block from the foreign file.
pub fn remove_entry_deployed(dest: &Path, entry: &store::DeployedEntry, module: &str) -> bool {
    match entry.mode {
        Ownership::Owned => {
            let ours = std::fs::read_link(dest)
                .map(|t| t.starts_with(store::gripsack_home()))
                .unwrap_or(false);
            ours && std::fs::remove_file(dest).is_ok()
        }
        Ownership::Merge => {
            let existing = std::fs::read_to_string(dest).unwrap_or_default();
            match crate::render::extract_block(&existing, module) {
                Some(content) if store::canonical_bytes_hash(content.as_bytes()) == entry.hash => {
                    let new =
                        crate::render::remove_block(&existing, module).expect("block found above");
                    if new.trim().is_empty() {
                        std::fs::remove_file(dest).is_ok()
                    } else {
                        store::atomic_write(dest, new.as_bytes()).is_ok()
                    }
                }
                _ => false, // drifted block is the user's now
            }
        }
        _ => {
            // copy-like: only delete bytes identical to what we wrote
            let Ok(current) = store::canonical_file_hash(dest) else {
                return false;
            };
            current == entry.hash && std::fs::remove_file(dest).is_ok()
        }
    }
}

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
                    let result = restore_entry(
                        &dest,
                        prev_entry,
                        &prev_state.expect("matched above").store_path,
                        name,
                    );
                    if result.is_ok() {
                        tracing::info!("restored {} (run rolled back)", entry.to);
                    }
                }
                None => {
                    if remove_entry_deployed(&dest, entry, name) {
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
        // install={} keys are payload-relative — a versioned top-level
        // dir in the archive must be part of the key; say what IS here
        let hint = std::fs::read_dir(store_path)
            .map(|entries| {
                let names: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                if names.is_empty() {
                    String::new()
                } else {
                    format!(" (payload top-level: {})", names.join(", "))
                }
            })
            .unwrap_or_default();
        return Err(fail(format!(
            "no payload or repo file at {} (from {}){hint}",
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
    // template payloads render at deploy time — the vars were computed
    // by the frontend at eval; the core only substitutes (0001 §3.7)
    let rendered = match &entry.mode {
        Ownership::Template => Some(crate::render::render_template(
            &std::fs::read(&source)?,
            &entry.vars,
            &entry.from,
        )?),
        _ => None,
    };
    let (summary, kind, hash) = match &entry.mode {
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
            // idempotent report (0014): a link already pointing at the
            // right store path is "unchanged", not "linked" — a mirror
            // swap that re-proves byte identity must not look like a
            // redeploy
            let already = std::fs::read_link(&dest)
                .map(|t| t == source)
                .unwrap_or(false);
            if !already {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                store::symlink_replace(&dest, &source)?;
            }
            let hash = store::canonical_file_hash(&source)?;
            if already {
                (
                    format!("{} unchanged", entry.to),
                    ReportKind::Satisfied,
                    hash,
                )
            } else {
                (
                    format!("linked {} → {}", from, entry.to),
                    ReportKind::Installed,
                    hash,
                )
            }
        }
        Ownership::TrackedCopy | Ownership::Template => {
            let content;
            let owned_bytes;
            if let Some(r) = &rendered {
                content = r.as_slice();
            } else {
                owned_bytes = std::fs::read(&source)?;
                content = owned_bytes.as_slice();
            }
            let hash = match &rendered {
                Some(bytes) => store::canonical_bytes_hash(bytes),
                None => store::canonical_file_hash(&source)?,
            };
            let prev_hash = prev
                .and_then(|m| m.entries.iter().find(|e| e.to == entry.to))
                .map(|e| e.hash.as_str());
            if dest.exists() {
                let current = store::canonical_file_hash(&dest)?;
                if current == hash {
                    (
                        format!("{} unchanged", entry.to),
                        ReportKind::Satisfied,
                        hash,
                    )
                } else if prev_hash == Some(current.as_str()) {
                    store::atomic_write(&dest, content)?;
                    (
                        format!("updated {} → {}", from, entry.to),
                        ReportKind::Configured,
                        hash,
                    )
                } else if ctx.take_over {
                    store::atomic_write(&dest, content)?;
                    (
                        format!("took over {} → {}", from, entry.to),
                        ReportKind::Configured,
                        hash,
                    )
                } else {
                    let note = if prev_hash.is_none() {
                        format!("{} exists (not deployed by gripsack) — kept", entry.to)
                    } else {
                        format!("{} drifted — kept", entry.to)
                    };
                    tracing::warn!("{}", note);
                    // the manifest must record what's ACTUALLY deployed
                    // (the kept content), not the source we declined to
                    // write — or drift resolution can never converge
                    // (e2e: drift_is_kept)
                    (note, ReportKind::Warned, current)
                }
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                store::atomic_write(&dest, content)?;
                (
                    format!("copied {} → {}", from, entry.to),
                    ReportKind::Configured,
                    hash,
                )
            }
        }
        Ownership::Merge => {
            // the file is foreign — we own exactly one delimited block
            // inside it and regenerate that block wholesale (conda's
            // replace-not-merge: drift inside the markers self-heals)
            let payload = std::fs::read_to_string(&source)
                .map_err(|e| fail(format!("cannot read {}: {e}", source.display())))?;
            let block = payload.trim_end_matches('\n');
            let hash = store::canonical_bytes_hash(block.as_bytes());
            let existing = std::fs::read_to_string(&dest).unwrap_or_default();
            let satisfied = crate::render::extract_block(&existing, module)
                .is_some_and(|c| store::canonical_bytes_hash(c.as_bytes()) == hash);
            if satisfied {
                (
                    format!("{} block unchanged", entry.to),
                    ReportKind::Satisfied,
                    hash,
                )
            } else {
                let new = crate::render::upsert_block(
                    &existing,
                    module,
                    &dest,
                    entry.marker.as_deref(),
                    &payload,
                )
                .map_err(fail)?;
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                store::atomic_write(&dest, new.as_bytes())?;
                (
                    format!("merged {} → {}", from, entry.to),
                    ReportKind::Configured,
                    hash,
                )
            }
        }
    };
    out.push(store::DeployedEntry {
        from: entry.from.clone(),
        to: entry.to.clone(),
        mode: entry.mode.clone(),
        vars: entry.vars.clone(),
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
