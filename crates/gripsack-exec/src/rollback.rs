//! User-initiated rollback through the SAME transaction protocol as
//! apply (plan/0025 §A): reconcile any interrupted run, declare the
//! run (marker with `RunOp::Rollback` — its commit condition
//! inverts), journal every destination mutation, flip, commit. A
//! crash mid-rollback is recovered by the next run's reconcile
//! exactly like a crashed apply, and an in-process failure restores
//! the pre-rollback generation's state before returning (0025 §D).

use crate::ctx::ExecError;
use crate::deploy::{dest_capability, journaled_computed, post_identity};
use gripsack_store as store;
use std::path::Path;
use store::journal::RunOp;

/// Roll back to `target`'s manifest: prune destinations the target
/// doesn't know (drift-guarded; take-over priors restore), restore
/// every entry the target deployed, render its env profile into the
/// generation, flip `current`, commit the journal. Returns human
/// notes (recovery lines, drift keeps) for the caller to surface.
///
/// Must run under the lifecycle lock.
pub fn rollback_generation(
    home_path: &Path,
    current: Option<&store::Generation>,
    target: &store::Generation,
) -> Result<Vec<String>, ExecError> {
    let home = gripsack_fs::open_or_create(home_path)?;
    // the clean-floor rule, same as apply: an interrupted run's
    // entries resolve BEFORE this run mutates anything
    let mut notes = store::journal::reconcile(&home)?;
    store::journal::begin_run(&home, target.number, RunOp::Rollback)?;

    let result = rollback_mutations(&home, home_path, current, target, &mut notes);
    match result {
        Ok(()) => {
            store::journal::commit_run(&home)?;
            Ok(notes)
        }
        Err(e) => {
            // ONE compensating path (0025 §D): reconcile the journal —
            // the priors this run captured ARE the pre-rollback state,
            // so an ordinary failure restores exactly what a kill
            // would restore on the next run. A reconcile failure
            // leaves the journal intact for that next run.
            match store::journal::reconcile(&home) {
                Ok(lines) => {
                    for line in lines {
                        tracing::info!("{line}");
                    }
                }
                Err(re) => {
                    tracing::warn!("rollback compensation failed (journal intact): {re}")
                }
            }
            Err(e)
        }
    }
}

/// The rollback's mutations, all journaled: prune-phase removals and
/// restores first, then the target generation's restores.
fn rollback_mutations(
    home: &gripsack_fs::Dir,
    home_path: &Path,
    current: Option<&store::Generation>,
    target: &store::Generation,
    notes: &mut Vec<String>,
) -> Result<(), ExecError> {
    // Remove destinations the target generation doesn't know about —
    // drift-guarded, same rule as apply's prune (user edits are never
    // deleted; merge entries lose only our block).
    if let Some(current) = current {
        for (name, state) in &current.modules {
            let target_entries = target.modules.get(name);
            for entry in &state.entries {
                let still = target_entries
                    .map(|s| s.entries.iter().any(|e| e.to == entry.to))
                    .unwrap_or(false);
                if still {
                    continue;
                }
                let dest = store::expand_home(&entry.to);
                let (dest_dir, dest_name) = dest_capability(&dest)?;
                // 0015 §4: an entry adopted with take-over gets its
                // ORIGINAL file back, not a deletion — journaled like
                // every other mutation now (0025 §A)
                let mut removed = false;
                journaled_computed(
                    home,
                    &dest_dir,
                    &dest_name,
                    &dest,
                    || {
                        removed =
                            crate::deploy::remove_or_restore_prior(&dest, entry, name, home_path);
                        Ok(())
                    },
                    || post_identity(&dest_dir, &dest_name),
                )?;
                if !removed {
                    notes.push(format!("kept {} — modified since deploy", entry.to));
                }
            }
        }
    }
    // Restore through the ONE deploy-restore path (0001 §3.5):
    // template re-renders with the recorded vars, merge re-upserts
    // only the block — never a naive byte copy.
    for (name, state) in &target.modules {
        for entry in &state.entries {
            let dest = store::expand_home(&entry.to);
            let (dest_dir, dest_name) = dest_capability(&dest)?;
            journaled_computed(
                home,
                &dest_dir,
                &dest_name,
                &dest,
                || crate::deploy::restore_entry(&dest, entry, &state.store_path, name),
                || post_identity(&dest_dir, &dest_name),
            )
            .map_err(|e| ExecError::Step {
                module: name.clone(),
                step: "rollback".into(),
                detail: format!("cannot restore {}: {e}", dest.display()),
            })?;
        }
    }
    // test-only kill switch: the restore→flip crash window's e2e (0025)
    crate::util::crash_hook("after-rollback-restore");
    // The env profile renders INTO the generation before the flip
    // (0025 §C): activation and profile become one indivisible step.
    crate::env::render_env_file(home_path, target.number, &target.modules)?;
    store::flip(home, home_path, target.number)?;
    Ok(())
}
