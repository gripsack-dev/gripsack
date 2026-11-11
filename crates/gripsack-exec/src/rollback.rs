//! User-initiated rollback through the SAME transaction protocol as
//! apply (plan/0025 §A), planned path-centrically (plan/0026 §1–2):
//! modules are an authoring concept; the transaction engine keys on
//! DESTINATIONS. Both manifests normalize to destination-keyed maps
//! and every destination gets exactly one transition, so a module
//! rename that keeps a destination can no longer journal it twice
//! (the second entry used to overwrite the first, losing the true
//! pre-rollback prior).
//!
//! Fidelity (plan/0056 K2, F2 step 8): every unit is decided by the
//! kernel's rollback machine (`restore`). A destination is restored to
//! exactly the identity the target generation recorded, from retained
//! bytes that reproduce it, and only when live state is absent or IS the
//! current generation's write (K1); live == target is a no-op; anything
//! else is user drift — kept and reported, never overwritten. A unit that
//! cannot be restored exactly refuses the rollback before the flip, and
//! the journal compensates what this run already wrote. Rollback
//! publishes no generation and writes nothing into the target: it
//! reselects it under a fresh transaction (K6, FND-2).

mod directory_transitions;
mod restore;

use crate::ctx::ExecError;
use gripsack_host::store;
use std::collections::BTreeMap;
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
    let home = gripsack_host::fs::open_or_create(home_path)?;
    // the clean-floor rule, same as apply: an interrupted run's
    // entries resolve BEFORE this run mutates anything
    let mut notes = store::journal::reconcile(&home, home_path)?;
    // durable activation resume (0032): same rule as apply — a
    // pending record naming the current generation re-runs its
    // intents; anything else is discarded, never run
    // same fail-closed rule as apply (0035 F5)
    let resumed = crate::activate::resume_activation(session, crate::progress::Reporter::silent()).map_err(|e| {
        ExecError::Step {
            output: "*".into(),
            step: "activate".into(),
            detail: format!(
                "activation recovery is blocked ({e}) — inspect $GRIPSACK_HOME/activation.json; refusing to mutate over it"
            ),
        }
    })?;
    notes.extend(resumed.into_iter().map(|r| store::journal::RecoveryNote {
        severity: if r.tone == crate::report::Tone::Warned {
            store::journal::NoteSeverity::Warn
        } else {
            store::journal::NoteSeverity::Info
        },
        message: format!("{}: {}", r.output, r.summary),
    }));
    let selection = store::journal::begin_run(
        &home,
        home_path,
        current.map(|g| g.number),
        target.number,
        RunOp::Rollback,
    )?;

    let result = (|| {
        // the kernel machine restores every unit and decides the flip
        // (plan/0056 F2 step 8); the target generation is reselected as
        // published, its profile included — nothing is written into it
        let (machine, reselect) = restore::until_flip(&selection, current, target, &mut notes)?;
        if reselect != *selection.identity() {
            return Err(ExecError::Step {
                output: "*".into(),
                step: "rollback".into(),
                detail: "the rollback machine reselected a selection this run did not reserve"
                    .into(),
            });
        }
        // the pending record lands BEFORE the flip (0032's shape,
        // 0037): a failure here compensates like any other; a crash
        // after the flip leaves the record for the next run's resume
        let intents = rollback_intents(current, target);
        let activation = store::activation::prepare(&home, &selection, intents)?;
        // test-only kill switch: the restore→flip crash window's e2e
        crate::util::crash_hook("after-rollback-restore");
        let committed = store::flip(selection)?;
        restore::finish(machine)?;
        Ok((activation, committed))
    })();
    match result {
        Ok((activation, committed)) => {
            // the flip already committed — a cleanup failure is
            // cleanup-pending, not a failed rollback (0030 §13); the
            // next reconcile finishes it
            if let Err(e) = store::journal::commit_run(committed) {
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
            if let Some(batch) = activation {
                match crate::activate::run(
                    batch,
                    &home,
                    home_path,
                    crate::progress::Reporter::silent(),
                    &crate::progress::VerificationLabels::new(),
                ) {
                    Ok(reports) => {
                        for report in reports {
                            notes.push(store::journal::RecoveryNote {
                                severity: if report.tone == crate::report::Tone::Warned {
                                    store::journal::NoteSeverity::Warn
                                } else {
                                    store::journal::NoteSeverity::Info
                                },
                                message: format!("{}: {}", report.output, report.summary),
                            });
                        }
                    }
                    Err(error) => notes.push(store::journal::RecoveryNote {
                        severity: store::journal::NoteSeverity::Warn,
                        message: format!(
                            "activation evidence retained ({error}) — inspect hooks before retrying"
                        ),
                    }),
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

/// Entries are keyed by physical destination AND managed-block identity.
/// Whole-file ownership still transfers across owner renames.
type DestMap<'m> = BTreeMap<store::OwnershipKey, (&'m str, &'m store::DeployedEntry, &'m Path)>;

fn by_destination(generation: &store::Generation) -> DestMap<'_> {
    let mut map = DestMap::new();
    for (name, state) in &generation.deployments {
        for entry in &state.entries {
            map.insert(
                entry.ownership_key(name),
                (name.as_str(), entry, &state.store_path),
            );
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
        for (name, state) in &current.deployments {
            if target.deployments.contains_key(name) {
                continue;
            }
            for record in &state.intents {
                if record.trigger == gripsack_host::store::ActivationTrigger::OnRemove {
                    intents.push(store::activation::PendingIntent {
                        output: name.clone(),
                        action: record.action.clone(),
                        trigger: record.trigger,
                    });
                }
            }
        }
    }
    intents.extend(crate::workspace::schedules::lifecycle::removal_intents(
        current,
        &target.deployments,
    ));
    intents
}

/// 0026 §9: never discover an incomplete target mid-mutation — every
/// store path and entry source must resolve before the first write.
fn preflight(target: &store::Generation) -> Result<(), ExecError> {
    for (name, state) in &target.deployments {
        if let Some(expected) = &state.tree256 {
            // content-addressed means the path NAMES the content —
            // prove it, don't trust it (0029 §8): a corrupted store
            // tree must never become the bytes rollback deploys
            let actual =
                store::canonical_tree_hash(&state.store_path).map_err(|e| ExecError::Step {
                    output: name.clone(),
                    step: "rollback".into(),
                    detail: format!(
                        "cannot verify generation {}'s store tree for {name}: {e}",
                        target.number
                    ),
                })?;
            if actual.as_str() != expected.as_str() {
                return Err(ExecError::Step {
                    output: name.clone(),
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
                output: name.clone(),
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
                    output: name.clone(),
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
