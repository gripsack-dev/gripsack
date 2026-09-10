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
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use store::journal::RunOp;

/// Roll back to `target`'s manifest. Returns typed recovery notes
/// (restores, drift keeps, skips) for the caller to surface by
/// severity. Requires a [`LifecycleSession`] (0045 F4): rollback
/// rewrites deployments and flips the generation pointer, so the
/// lock is part of the signature, not prose.
pub fn rollback_generation(
    session: &crate::LifecycleSession,
    current: Option<&store::Generation>,
    target: &store::Generation,
) -> Result<Vec<store::journal::RecoveryNote>, ExecError> {
    let home_path = session.home();
    let home = gripsack_fs::open_or_create(home_path)?;
    // the clean-floor rule, same as apply: an interrupted run's
    // entries resolve BEFORE this run mutates anything
    let mut notes = store::journal::reconcile(&home, home_path)?;
    // durable activation resume (0032): same rule as apply — a
    // pending record naming the current generation re-runs its
    // intents; anything else is discarded, never run
    // same fail-closed rule as apply (0035 F5)
    let resumed = crate::activate::resume_pending(&home, current.map(|g| g.number)).map_err(|e| {
        ExecError::Step {
            module: "*".into(),
            step: "activate".into(),
            detail: format!(
                "the activation record is unreadable ({e}) — inspect $GRIPSACK_HOME/activation.json; refusing to mutate over it"
            ),
        }
    })?;
    notes.extend(resumed.into_iter().map(|r| store::journal::RecoveryNote {
        severity: if r.kind == crate::report::ReportKind::Warned {
            store::journal::NoteSeverity::Warn
        } else {
            store::journal::NoteSeverity::Info
        },
        message: format!("{}: {}", r.module, r.summary),
    }));
    store::journal::begin_run(
        &home,
        current.map(|g| g.number),
        target.number,
        RunOp::Rollback,
    )?;

    let result = (|| {
        let (ops, mut planned_notes) = plan(home_path, current, target)?;
        execute(&home, home_path, &ops)?;
        notes.append(&mut planned_notes);
        // The env profile renders INTO the generation before the flip
        // (0025 §C): activation and profile become one indivisible step.
        crate::env::render_env_file(home_path, target.number, &target.modules)?;
        // the pending record lands BEFORE the flip (0032's shape,
        // 0037): a failure here compensates like any other; a crash
        // after the flip leaves the record for the next run's resume
        let intents = rollback_intents(current, target);
        if !intents.is_empty() {
            store::activation::write_pending(
                &home,
                &store::activation::PendingActivation {
                    generation: target.number,
                    intents,
                },
            )?;
        }
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
            // activate the TARGET generation as recorded (0037): the
            // pending record is the source (it also carries the
            // removal hooks of modules this rollback undeclared)
            if let Some(pending) = store::activation::read_pending(&home)? {
                for report in crate::activate::run(&pending.intents) {
                    notes.push(store::journal::RecoveryNote {
                        severity: if report.kind == crate::report::ReportKind::Warned {
                            store::journal::NoteSeverity::Warn
                        } else {
                            store::journal::NoteSeverity::Info
                        },
                        message: format!("{}: {}", report.module, report.summary),
                    });
                }
                if let Err(e) = store::activation::clear_pending(&home) {
                    notes.push(store::journal::RecoveryNote {
                        severity: store::journal::NoteSeverity::Warn,
                        message: format!(
                            "activation record cleanup pending ({e}) — the next run finishes it"
                        ),
                    });
                }
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

/// `(module, entry, store_path)` per CANONICAL destination (0035 F1).
type DestMap<'m> = BTreeMap<std::path::PathBuf, (&'m str, &'m store::DeployedEntry, &'m Path)>;

fn by_destination(generation: &store::Generation) -> DestMap<'_> {
    // E111 guarantees one deployer per destination within a
    // generation; first-wins is defensive against hand-edited
    // manifests
    let mut map = DestMap::new();
    for (name, state) in &generation.modules {
        for entry in &state.entries {
            map.entry(entry.key())
                .or_insert((name.as_str(), entry, &state.store_path));
        }
    }
    map
}

/// The intents a rollback activates (0037): the target generation's
/// recorded intents, plus the on_remove hooks of modules the rollback
/// undeclares.
fn rollback_intents(
    current: Option<&store::Generation>,
    target: &store::Generation,
) -> Vec<store::activation::PendingIntent> {
    let mut intents = crate::activate::collect_from_manifest(target);
    if let Some(current) = current {
        for (name, state) in &current.modules {
            if target.modules.contains_key(name) {
                continue;
            }
            for record in &state.intents {
                if record.trigger == gripsack_ir::Trigger::OnRemove {
                    intents.push(store::activation::PendingIntent {
                        module: name.clone(),
                        action: record.action.clone(),
                        trigger: record.trigger,
                    });
                }
            }
        }
    }
    intents
}

/// Plan one op per destination, with drift policy and preflight
/// (0026 §1, §9). The decisions are the SAME planner apply and the
/// CLI preview share (0034): rollback is plan_entry_op with the
/// target generation's manifest as the desired state — plan_copy's
/// three-way IS the rollback drift rule (live == target → nothing;
/// live == current's record → restore; else → keep, drift preserved).
fn plan(
    home_path: &Path,
    current: Option<&store::Generation>,
    target: &store::Generation,
) -> Result<(Vec<crate::ops::Op>, Vec<store::journal::RecoveryNote>), ExecError> {
    use store::journal::{NoteSeverity, RecoveryNote};
    preflight(target)?;
    let current_by_dest = current.map(by_destination).unwrap_or_default();
    let target_by_dest = by_destination(target);
    let dests: BTreeSet<std::path::PathBuf> = current_by_dest
        .keys()
        .chain(target_by_dest.keys())
        .cloned()
        .collect();
    let mut ops = Vec::new();
    let mut notes = Vec::new();
    for dest in dests {
        match (current_by_dest.get(&dest), target_by_dest.get(&dest)) {
            // only the current generation deploys it: the prune rule
            (Some((name, entry, sp)), None) => {
                // preserved drift was never written by gripsack —
                // rollback and prune never touch it (0029 §2)
                if entry.preserved_drift {
                    continue;
                }
                match crate::ops::plan_remove_op(name, entry, sp, home_path)? {
                    None => {} // already gone
                    Some(op) if matches!(op.kind(), crate::ops::OpKind::Preserved) => {
                        notes.push(RecoveryNote {
                            severity: NoteSeverity::Warn,
                            message: format!(
                                "kept {} — drifted since the current generation; your edit stands",
                                op.dest().display()
                            ),
                        });
                    }
                    Some(op) => ops.push(op),
                }
            }
            // only the target deploys it: restore, unless foreign
            // content stands there now (drift preserved)
            (None, Some((name, entry, sp))) => {
                if entry.preserved_drift {
                    continue;
                }
                match crate::ops::plan_restore_op(name, entry, sp, None, home_path)? {
                    None => notes.push(RecoveryNote {
                        severity: NoteSeverity::Warn,
                        message: format!(
                            "skipped {} — no safe restore plan (stale manifest or unreadable merge file)",
                            dest.display()
                        ),
                    }),
                    Some(op) => match op.kind() {
                        crate::ops::OpKind::Satisfied => {} // already there
                        crate::ops::OpKind::Preserved => notes.push(RecoveryNote {
                            severity: NoteSeverity::Warn,
                            message: format!(
                                "kept {} — foreign content stands there; your edit stands",
                                op.dest().display()
                            ),
                        }),
                        _ => ops.push(op),
                    },
                }
            }
            // both deploy it: restore only from a clean base
            (Some((_cname, centry, _csp)), Some((tname, tentry, tsp))) => {
                // either side marked preserved-drift means gripsack is
                // not the writer — never restore over it (0029 §2)
                if centry.preserved_drift || tentry.preserved_drift {
                    continue;
                }
                // plan_copy's three-way through the one planner: the
                // target's record is the desired state, the current's
                // is the lineage — live == target is a no-op, live ==
                // current restores, anything else keeps the drift
                match crate::ops::plan_restore_op(tname, tentry, tsp, Some(centry), home_path)? {
                    None => notes.push(RecoveryNote {
                        severity: NoteSeverity::Warn,
                        message: format!(
                            "skipped {} — no safe restore plan (stale manifest or unreadable merge file)",
                            dest.display()
                        ),
                    }),
                    Some(op) => match op.kind() {
                        crate::ops::OpKind::Satisfied => {}
                        crate::ops::OpKind::Preserved => notes.push(RecoveryNote {
                            severity: NoteSeverity::Warn,
                            message: format!(
                                "kept {} — drifted since the current generation; your edit stands",
                                op.dest().display()
                            ),
                        }),
                        _ => ops.push(op),
                    },
                }
            }
            (None, None) => {} // unreachable, union of keys
        }
    }
    Ok((ops, notes))
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
    ops: &[crate::ops::Op],
) -> Result<(), ExecError> {
    for op in ops {
        crate::ops::execute_op(home, home_path, op.as_executable()?)?;
    }
    Ok(())
}
