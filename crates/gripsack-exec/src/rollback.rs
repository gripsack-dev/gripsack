//! User-initiated rollback through the SAME transaction protocol as
//! apply (plan/0025 §A), planned path-centrically (plan/0026 §1–2):
//! modules are an authoring concept; the transaction engine keys on
//! DESTINATIONS. Both manifests normalize to destination-keyed maps
//! and every destination gets exactly one transition, so a module
//! rename that keeps a destination can no longer journal it twice
//! (the second entry used to overwrite the first, losing the true
//! pre-rollback prior).
//!
//! Drift policy (0026 §1): a destination shared by both generations
//! is restored only when live state IS the current generation's
//! deployment; live == target is a no-op; anything else is user
//! drift — preserved and reported, never overwritten.

use crate::ctx::ExecError;
use crate::deploy::{
    RestorePlan, compute_restore, dest_capability, execute_restore, intact_deployed, journaled,
    prune_intent, remove_or_restore_prior,
};
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use store::journal::RunOp;

/// Roll back to `target`'s manifest. Returns typed recovery notes
/// (restores, drift keeps, skips) for the caller to surface by
/// severity. Must run under the lifecycle lock.
pub fn rollback_generation(
    home_path: &Path,
    current: Option<&store::Generation>,
    target: &store::Generation,
) -> Result<Vec<store::journal::RecoveryNote>, ExecError> {
    let home = gripsack_fs::open_or_create(home_path)?;
    // the clean-floor rule, same as apply: an interrupted run's
    // entries resolve BEFORE this run mutates anything
    let mut notes = store::journal::reconcile(&home, home_path)?;
    store::journal::begin_run(
        &home,
        current.map(|g| g.number),
        target.number,
        RunOp::Rollback,
    )?;

    let result = (|| {
        let transitions = plan(home_path, current, target)?;
        execute(&home, home_path, &transitions, &mut notes)?;
        // The env profile renders INTO the generation before the flip
        // (0025 §C): activation and profile become one indivisible step.
        crate::env::render_env_file(home_path, target.number, &target.modules)?;
        // test-only kill switch: the restore→flip crash window's e2e
        crate::util::crash_hook("after-rollback-restore");
        store::flip(&home, home_path, target.number)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            // the flip already committed — a cleanup failure is
            // cleanup-pending, not a failed rollback (0030 §13); the
            // next reconcile finishes it
            if let Err(e) = store::journal::commit_run(&home) {
                notes.push(store::journal::RecoveryNote {
                    severity: store::journal::NoteSeverity::Warn,
                    message: format!(
                        "generation {} active; journal cleanup pending ({e}) — the next run finishes it",
                        target.number
                    ),
                });
            }
            Ok(notes)
        }
        Err(e) => {
            // ONE compensating path (0025 §D): reconcile the journal —
            // the priors this run captured ARE the pre-rollback state,
            // so an ordinary failure restores exactly what a kill
            // would restore on the next run. A reconcile failure
            // leaves the journal intact for that next run.
            match store::journal::reconcile(&home, home_path) {
                Ok(lines) => notes.extend(lines),
                Err(re) => {
                    tracing::warn!("rollback compensation failed (journal intact): {re}")
                }
            }
            Err(e)
        }
    }
}

/// One destination's planned transition.
enum Transition {
    /// Only the current generation deploys it: drift-guarded removal
    /// / prior restore, intent recorded up front.
    Remove {
        module: String,
        entry: store::DeployedEntry,
        intent: store::journal::Intended,
        /// the live identity the intact check ran against (0029 §3)
        expected: crate::deploy::Expect,
        /// the current generation's store path (exact removal guard)
        store_path: PathBuf,
    },
    /// Restore the target generation's deployment.
    Restore {
        plan: RestorePlan,
        /// the live identity the drift decision ran against
        expected: crate::deploy::Expect,
    },
    /// Live state already matches the target — nothing to do.
    Noop,
    /// The target asks for this destination but no safe restore could
    /// be constructed — surfaced in the rollback output, never
    /// silently skipped (0030 §H8).
    Skipped,
    /// Drifted: matches neither current nor target — the user's now.
    Keep,
}

/// `(module, entry, store_path)` per destination.
type DestMap<'m> = BTreeMap<&'m str, (&'m str, &'m store::DeployedEntry, &'m Path)>;

fn by_destination(generation: &store::Generation) -> DestMap<'_> {
    // E111 guarantees one deployer per destination within a
    // generation; first-wins is defensive against hand-edited
    // manifests
    let mut map = DestMap::new();
    for (name, state) in &generation.modules {
        for entry in &state.entries {
            map.entry(entry.to.as_str())
                .or_insert((name.as_str(), entry, &state.store_path));
        }
    }
    map
}

/// The destination's live identity in intent terms (0026 §1): link
/// target for owned, canonical bytes hash for copy/template, the
/// module's block hash for merge. None when absent (or, for merge,
/// when the block is absent).
fn live_intent_identity(
    dest: &Path,
    entry: &store::DeployedEntry,
    module: &str,
) -> std::io::Result<Option<String>> {
    // NotFound-only-is-absent (0027 §7): an unreadable destination is
    // not "absent", "unchanged", or "drifted" — it is an error, and
    // the rollback aborts before mutating on top of the unknown
    let read_text = |d: &Path| match std::fs::read_to_string(d) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    };
    match entry.mode {
        gripsack_ir::Ownership::Owned => {
            let target = match std::fs::read_link(dest) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            Ok(Some(target.to_string_lossy().into_owned()))
        }
        gripsack_ir::Ownership::Merge => Ok(read_text(dest)?
            .and_then(|text| crate::template::extract_block(&text, module))
            .map(|block| store::canonical_bytes_hash(block.as_bytes()).to_string())),
        // the manifest domain is mode-aware for tracked copies (0031):
        // a chmodded copy is drift, not intact
        gripsack_ir::Ownership::TrackedCopy => {
            let bytes = match std::fs::read(dest) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::MetadataExt;
                std::fs::metadata(dest)?.mode() & 0o7777
            };
            #[cfg(not(unix))]
            let mode = 0o644;
            Ok(Some(
                store::canonical_bytes_identity(&bytes, mode).to_string(),
            ))
        }
        // templates: bytes-only (a rendered file's mode is unmanaged)
        gripsack_ir::Ownership::Template => {
            let bytes = match std::fs::read(dest) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            Ok(Some(store::canonical_bytes_hash(&bytes).to_string()))
        }
    }
}

/// Plan one transition per destination, with drift policy and
/// preflight (0026 §1, §9).
fn plan(
    home_path: &Path,
    current: Option<&store::Generation>,
    target: &store::Generation,
) -> Result<Vec<(PathBuf, Transition)>, ExecError> {
    preflight(target)?;
    let current_by_dest = current.map(by_destination).unwrap_or_default();
    let target_by_dest = by_destination(target);
    let dests: BTreeSet<&str> = current_by_dest
        .keys()
        .chain(target_by_dest.keys())
        .copied()
        .collect();
    let mut out = Vec::new();
    for dest in dests {
        let dest_path = store::expand_home(dest);
        let transition = match (current_by_dest.get(dest), target_by_dest.get(dest)) {
            // only the current generation deploys it: the prune rule
            (Some((name, entry, sp)), None) => {
                // preserved drift was never written by gripsack —
                // rollback and prune never touch it (0029 §2)
                if entry.preserved_drift || dest_path.symlink_metadata().is_err() {
                    // preserved drift: never written by gripsack, never
                    // touched (0029 §2). Absent: already gone.
                    Transition::Noop
                } else if entry.mode == gripsack_ir::Ownership::Merge {
                    // merge intactness is the BLOCK's hash (the file
                    // is foreign), and removal splices the block out —
                    // the intent is the resulting content, like apply's
                    // merge prune
                    // NotFound-only-is-absent (0030 §H7)
                    let existing = match std::fs::read_to_string(&dest_path) {
                        Ok(t) => t,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                        Err(e) => return Err(e.into()),
                    };
                    match crate::template::extract_block(&existing, name) {
                        Some(content)
                            if store::canonical_bytes_hash(content.as_bytes()).as_str()
                                == entry.hash =>
                        {
                            let new = crate::template::remove_block(&existing, name)
                                .expect("block found above");
                            // the splice preserves the foreign file's
                            // mode (0026 §7) — intents are mode-aware
                            #[cfg(unix)]
                            let splice_mode = {
                                use std::os::unix::fs::MetadataExt;
                                std::fs::metadata(&dest_path)
                                    .map(|m| m.mode() & 0o7777)
                                    .unwrap_or(0o644)
                            };
                            #[cfg(not(unix))]
                            let splice_mode = 0o644;
                            let intent = if new.trim().is_empty() {
                                store::journal::Intended::Removed
                            } else {
                                store::journal::Intended::Object(
                                    store::journal::ObjectIdentity::File(
                                        store::canonical_bytes_identity(
                                            new.as_bytes(),
                                            splice_mode,
                                        ),
                                    ),
                                )
                            };
                            Transition::Remove {
                                module: name.to_string(),
                                entry: (*entry).clone(),
                                intent,
                                expected: Some(store::journal::ObjectIdentity::File(
                                    store::canonical_bytes_identity(
                                        existing.as_bytes(),
                                        splice_mode,
                                    ),
                                )),
                                store_path: sp.to_path_buf(),
                            }
                        }
                        _ => Transition::Keep, // block drifted or gone
                    }
                } else if intact_deployed(&dest_path, entry, sp) {
                    let (dest_dir, dest_name) = dest_capability(&dest_path)?;
                    Transition::Remove {
                        module: name.to_string(),
                        entry: (*entry).clone(),
                        intent: prune_intent(entry, home_path)?,
                        expected: store::journal::live_identity(&dest_dir, &dest_name)?,
                        store_path: sp.to_path_buf(),
                    }
                } else {
                    Transition::Keep // drifted — the user's now
                }
            }
            // only the target deploys it: restore — unless foreign
            // content stands there now (drift preserved)
            (None, Some((name, entry, store_path))) => {
                if entry.preserved_drift {
                    // the target never deployed this — nothing to restore
                    Transition::Noop
                } else {
                    match compute_restore(&dest_path, entry, store_path, name)? {
                        // None = could not construct a safe restore —
                        // SURFACED, never a silent no-op (0030 §H8)
                        None => Transition::Skipped,
                        Some(plan) => {
                            // the manifest's record IS the
                            // manifest-domain target — every mode
                            let target_id = entry.hash.clone();
                            let live = live_intent_identity(&dest_path, entry, name)?;
                            match live {
                                None => Transition::Restore {
                                    plan,
                                    expected: None,
                                },
                                Some(l) if l == target_id => Transition::Noop,
                                Some(_) => Transition::Keep,
                            }
                        }
                    }
                }
            }
            // both deploy it: restore only from a clean base
            (Some((cname, centry, csp)), Some((tname, tentry, tsp))) => {
                // either side marked preserved-drift means gripsack is
                // not the writer — never restore over it (0029 §2)
                if centry.preserved_drift || tentry.preserved_drift {
                    out.push((dest_path, Transition::Noop));
                    continue;
                }
                let target_plan = compute_restore(&dest_path, tentry, tsp, tname)?;
                let live = live_intent_identity(&dest_path, centry, cname)?;
                match (live, target_plan) {
                    (_, None) => Transition::Skipped,
                    (None, Some(plan)) => Transition::Restore {
                        plan,
                        expected: None,
                    },
                    (Some(live), Some(plan)) => {
                        // the manifest records ARE the
                        // manifest-domain identities (merge's are
                        // block hashes; the intact check below reads
                        // the live object in the same domain)
                        let (current_id, target_id) = (centry.hash.clone(), tentry.hash.clone());
                        // but the current generation's restorability
                        // must still be PROVEN — its store source
                        // could be missing (gc'd, corrupt)
                        if compute_restore(&dest_path, centry, csp, cname)?.is_none() {
                            out.push((dest_path, Transition::Keep));
                            continue;
                        }
                        if live == target_id {
                            Transition::Noop
                        } else if live == current_id {
                            // the precondition checks the live
                            // OBJECT, typed (merge's `live` above was
                            // the block hash — the precondition needs
                            // the whole file, mode-aware)
                            let (dest_dir, dest_name) = dest_capability(&dest_path)?;
                            let expected: crate::deploy::Expect =
                                store::journal::live_identity(&dest_dir, &dest_name)?;
                            Transition::Restore { plan, expected }
                        } else {
                            Transition::Keep
                        }
                    }
                }
            }
            (None, None) => Transition::Noop, // unreachable, union of keys
        };
        out.push((dest_path, transition));
    }
    Ok(out)
}

/// 0026 §9: never discover an incomplete target mid-mutation — every
/// store path and entry source must resolve before the first write.
fn preflight(target: &store::Generation) -> Result<(), ExecError> {
    for (name, state) in &target.modules {
        if let Some(expected) = &state.tree256 {
            // content-addressed means the path NAMES the content —
            // prove it, don't trust it (0029 §8): a corrupted store
            // tree must never become the bytes rollback deploys
            let actual =
                store::canonical_tree_hash(&state.store_path).map_err(|e| ExecError::Step {
                    module: name.clone(),
                    step: "rollback".into(),
                    detail: format!(
                        "cannot verify generation {}'s store tree for {name}: {e}",
                        target.number
                    ),
                })?;
            if actual.as_str() != expected.as_str() {
                return Err(ExecError::Step {
                    module: name.clone(),
                    step: "rollback".into(),
                    detail: format!(
                        "store tree for {name} no longer matches generation {}'s recorded \
                         identity (tree256) — refusing to roll back; `grip store verify` for detail",
                        target.number
                    ),
                });
            }
        }
        if !state.store_path.is_dir() {
            return Err(ExecError::Step {
                module: name.clone(),
                step: "rollback".into(),
                detail: format!(
                    "generation {} is incomplete: {} is missing (corrupt or gc'd store) — \
                     refusing to roll back",
                    target.number,
                    state.store_path.display()
                ),
            });
        }
        for entry in &state.entries {
            let src = state.store_path.join(&entry.from);
            if !src.exists() {
                return Err(ExecError::Step {
                    module: name.clone(),
                    step: "rollback".into(),
                    detail: format!(
                        "generation {} is incomplete: {} is missing — refusing to roll back",
                        target.number,
                        src.display()
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Execute the plan: every mutation journaled, one transition per
/// destination.
fn execute(
    home: &gripsack_fs::Dir,
    home_path: &Path,
    transitions: &[(PathBuf, Transition)],
    notes: &mut Vec<store::journal::RecoveryNote>,
) -> Result<(), ExecError> {
    for (dest, transition) in transitions {
        match transition {
            Transition::Noop => {}
            Transition::Skipped => {
                notes.push(store::journal::RecoveryNote {
                    severity: store::journal::NoteSeverity::Warn,
                    message: format!(
                        "skipped {} — no safe restore plan (stale manifest or unreadable merge file)",
                        dest.display()
                    ),
                });
            }
            Transition::Keep => {
                notes.push(store::journal::RecoveryNote {
                    severity: store::journal::NoteSeverity::Warn,
                    message: format!(
                        "kept {} — drifted since the current generation; your edit stands",
                        dest.display()
                    ),
                });
            }
            Transition::Remove {
                module,
                entry,
                intent,
                expected,
                store_path,
            } => {
                let (dest_dir, dest_name) = dest_capability(dest)?;
                journaled(
                    home,
                    &dest_dir,
                    &dest_name,
                    dest,
                    intent.clone(),
                    expected.clone(),
                    || {
                        // 0027 §1: a failed removal/restore aborts the
                        // rollback — the flip never commits it
                        remove_or_restore_prior(
                            &dest_dir, &dest_name, entry, module, home_path, store_path,
                        )?;
                        Ok(())
                    },
                )?;
            }
            Transition::Restore { plan, expected } => {
                let (dest_dir, dest_name) = dest_capability(dest)?;
                journaled(
                    home,
                    &dest_dir,
                    &dest_name,
                    dest,
                    plan.intent.clone(),
                    expected.clone(),
                    || execute_restore(&dest_dir, &dest_name, plan),
                )?;
            }
        }
    }
    Ok(())
}
