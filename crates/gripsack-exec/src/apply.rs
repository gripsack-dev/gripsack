//! `apply` — the lifecycle: order, execute, compare, flip (0001 §4).

use crate::ctx::Outcome;
use crate::ctx::{Ctx, ExecError};
use crate::report::ApplyResult;
use gripsack_ir::Ir;
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use tracing::{info, info_span};

/// Apply selected workspace profiles and activate one new generation.
/// The lifecycle — read current → build → flip — is one
/// read-modify-write, so the whole run holds `$GRIPSACK_HOME/
/// locks/apply.flock` (finding A): two concurrent applies serialize,
/// never lose a manifest update.
pub fn apply(ir: &Ir, ctx: &Ctx) -> Result<ApplyResult, ExecError> {
    if let Some(diagnostic) =
        ir.workspace_execution_error(gripsack_ir::workspace::WorkspaceOperation::Apply)
    {
        return Err(ExecError::Gate(diagnostic));
    }
    let (session, native) = crate::workspace::prepare_apply(ir, ctx)?;
    apply_prepared(ctx, session, native)
}

pub(crate) fn apply_in_session(
    ir: &Ir,
    ctx: &Ctx,
    session: crate::LifecycleSession,
) -> Result<ApplyResult, ExecError> {
    if let Some(diagnostic) =
        ir.workspace_execution_error(gripsack_ir::workspace::WorkspaceOperation::Apply)
    {
        return Err(ExecError::Gate(diagnostic));
    }
    let (session, native) = crate::workspace::prepare_apply_in_session(ir, ctx, session)?;
    apply_prepared(ctx, session, native)
}

fn apply_prepared(
    ctx: &Ctx,
    session: crate::LifecycleSession,
    native: crate::workspace::NativeProfiles,
) -> Result<ApplyResult, ExecError> {
    // crash recovery (0019): a previous run killed between a deploy
    // mutation and the flip left uncommitted journal entries — the
    // filesystem sits between generations. Restore the priors before
    // deploying anything; the run then proceeds from a clean floor.
    let recovered = store::journal::reconcile(ctx.home_dir()?, &ctx.home)?;
    let mut reports = Vec::new();
    if !recovered.is_empty() {
        reports.push(crate::report::StepReport {
            output: "*".into(),
            summary: format!(
                "recovered {} destination(s) from an interrupted run",
                recovered.len()
            ),
            kind: crate::report::ReportKind::Warned,
        });
        for line in &recovered {
            tracing::warn!("{line}");
        }
    }

    // The previous generation's state, for drift detection — distinct
    // from the manifest being built. Read once: prune and the
    // satisfied comparison below reuse it.
    let current_gen = store::current_generation(&ctx.home)?;
    // durable activation resume (0032): a previous run killed around
    // its adapters left a pending record — run or discard it BEFORE
    // this run mutates anything (a satisfied run resumes too)
    // invalid pending metadata BLOCKS the run (0035 F5): mutating
    // over an unreadable activation record would strand the pending
    // generation's intents — same rule as the journal's quarantine
    let resumed = crate::activate::resume_activation(&session).map_err(|e| {
        ExecError::Step {
            output: "*".into(),
            step: "activate".into(),
            detail: format!(
                "activation recovery is blocked ({e}) — inspect $GRIPSACK_HOME/activation.json; refusing to mutate over it"
            ),
        }
    })?;
    reports.extend(resumed);
    // Fault-injection harness seam, available in the same build profiles as
    // crash_hook: execute real recovery/resume, then stop before allocating or
    // applying a new generation. Ordinary apply still checks retained content.
    if std::env::var_os("GRIPSACK_FS_RECOVER_ONLY").is_some() {
        return Ok(ApplyResult {
            outcome: Outcome::Satisfied {
                generation: current_gen,
            },
            reports,
        });
    }
    // the allocator is NOT current+1 (0026 §3): after a rollback,
    // current is lower than the highest generation on disk, and
    // reusing a number would rewrite immutable history. Allocate
    // above every generation ever published (gc'd or not — gc never
    // removes the current generation, and the tip stays on disk
    // while it is current).
    let next_gen = store::generations::allocate(&ctx.home, ctx.home_dir()?)?;
    // a known current generation with an unreadable manifest is
    // categorically different from no current generation (0027 §3):
    // planning without the authoritative previous state would skip
    // prunes and mis-plan ownership — block the mutation
    let prev_manifest: Option<store::Generation> = match current_gen {
        Some(n) => Some(
            store::generations::admit_manifest(&ctx.home, n).map_err(|e| ExecError::Step {
                output: "*".into(),
                step: "manifest".into(),
                detail: format!(
                    "current generation {n}'s manifest is unreadable ({e}) — refusing to apply"
                ),
            })?,
        ),
        None => None,
    };
    let mut migration = native.migration(&ctx.home, current_gen, prev_manifest.as_ref())?;
    let prev_deployments: BTreeMap<String, store::DeploymentState> = prev_manifest
        .as_ref()
        .map(|g| g.deployments.clone())
        .unwrap_or_default();
    // A subset apply starts from the current generation's manifest and
    // replaces only the profiles it touches. A full apply reconciles:
    // profiles absent from the workspace are dropped from the
    // manifest — prune_undeclared then removes their destinations.
    let mut deployments: BTreeMap<String, store::DeploymentState> = if ctx.only.is_empty() {
        BTreeMap::new()
    } else {
        prev_deployments.clone()
    };
    if let Some(migration) = &migration {
        migration.release_entries(&mut deployments);
    }
    // declare the generation this run builds BEFORE any mutation:
    // recovery compares the marker against `current` — a crash after
    // the flip but before journal cleanup must read as COMMITTED,
    // never restore priors the new generation owns (review 5.1)
    let selection = store::journal::begin_run(
        ctx.home_dir()?,
        &ctx.home,
        current_gen,
        next_gen,
        store::journal::RunOp::Apply,
    )?;
    let execution = (|| {
        if let Some(previous) = &prev_manifest {
            for receipt in &previous.directory_transitions {
                store::journal::directory::bind_existing(&selection, receipt)?;
            }
        }
        native.deploy(ctx, &selection, &prev_deployments, migration.as_mut())
    })();
    let outcome = match execution {
        Ok(outcome) => outcome,
        Err(error) => {
            compensate(ctx);
            return Err(error);
        }
    };
    // Recovery reports describe an earlier committed selection. They remain
    // visible, but cannot manufacture a fresh deployment or hook instance.
    let deployment_reports_start = reports.len();
    for (name, module_reports) in outcome.reports {
        let span = info_span!("profile", name = name.as_str());
        let _entered = span.enter();
        reports.extend(module_reports);
    }
    deployments.extend(outcome.deployments);
    if let Some(prev) = &prev_manifest {
        inherit_priors(prev, &mut deployments);
    }
    let next = next_gen;

    // Everything between profile deployment and the flip is recoverable
    // through ONE compensating path (0025 §D): an ordinary error —
    // prune, manifest, env render — reconciles exactly like
    // a kill would have. The flip itself is one atomic rename: an
    // error from it means nothing flipped, so it compensates too.
    match pre_flip(
        ctx,
        &session,
        &selection,
        &prev_manifest,
        &deployments,
        next,
        &reports[deployment_reports_start..],
        &native,
    ) {
        Ok(Some(_generation)) => {}
        // vacuous run: nothing journaled, nothing flipped — the
        // marker begin_run wrote must not linger
        Ok(None) => {
            store::journal::end_run(selection)?;
            return Ok(ApplyResult {
                outcome: Outcome::Satisfied {
                    generation: current_gen,
                },
                reports,
            });
        }
        Err(e) => {
            compensate(ctx);
            return Err(e);
        }
    };
    // the pending activation record lands BEFORE the flip (0032): a
    // kill anywhere now leaves either a committed generation with its
    // intents recorded (next run resumes) or a record naming a
    // generation that never committed (next run discards)
    let mut intents = native.activation_intents();
    // on_remove hooks: a profile dropped from the workspace fires
    // its removal intents with the new generation — the record is the
    // only durable place the old intents live
    if let Some(prev) = &prev_manifest {
        for (name, state) in &prev.deployments {
            if deployments.contains_key(name) {
                continue;
            }
            for record in &state.intents {
                if record.trigger == gripsack_store::ActivationTrigger::OnRemove {
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
        prev_manifest.as_ref(),
        &deployments,
    ));
    crate::workspace::schedules::lifecycle::registration_intents(&deployments, &mut intents);
    let activation = match store::activation::prepare(ctx.home_dir()?, &selection, intents) {
        Ok(activation) => activation,
        Err(error) => {
            // The immutable plan/pointer is still inside the transaction.
            compensate(ctx);
            return Err(error.into());
        }
    };
    let committed = match store::flip(selection) {
        Ok(committed) => committed,
        Err(error) => {
            compensate(ctx);
            return Err(error.into());
        }
    };
    // test-only kill switch: the flip→adapters crash window's e2e
    crate::util::crash_hook("before-adapters");
    // the flip is the run's commit point: everything the journal
    // recorded is now owned by the new generation — the crash window
    // closes here
    if let Err(e) = store::journal::commit_run(committed) {
        // the flip already committed — this is cleanup-pending, not a
        // failed apply (0029 §13); the next run's reconcile finishes it
        reports.push(crate::report::StepReport {
            output: "*".into(),
            summary: format!(
                "generation {next} active; journal cleanup pending ({e}) — the next apply finishes it"
            ),
            kind: crate::report::ReportKind::Warned,
        });
    }
    if let Some(batch) = activation {
        match crate::activate::run(batch, ctx.home_dir()?, &ctx.home) {
            Ok(outcomes) => reports.extend(outcomes),
            Err(error) => reports.push(crate::report::StepReport {
                output: "*".into(),
                summary: format!("generation {next} active; activation evidence retained ({error}) — inspect hooks before retrying"),
                kind: crate::report::ReportKind::Warned,
            }),
        }
    }
    info!(generation = %next, "activated");
    Ok(ApplyResult {
        outcome: Outcome::Applied { generation: next },
        reports,
    })
}

/// Ownership lineage (0029 §1): an origin prior rides the whole
/// epoch — every generation carries it forward per destination until
/// a successful restore or prune ends the epoch. Keyed by
/// DESTINATION (a deployment rename transfers the origin to the new
/// deployment rather than losing it).
///
/// The inherited origin ALWAYS wins (0030 §H5): a second
/// `--take-over` captures the drifted bytes for the crash-recovery
/// journal, but the manifest keeps the epoch's FIRST origin —
/// undeclare must restore the pre-adoption state, not the last
/// absorbed drift.
fn inherit_priors(
    prev: &store::Generation,
    deployments: &mut BTreeMap<String, store::DeploymentState>,
) {
    let mut by_dest: std::collections::BTreeMap<std::path::PathBuf, &store::Prior> =
        Default::default();
    for state in prev.deployments.values() {
        for entry in &state.entries {
            if let Some(prior) = &entry.prior {
                by_dest.entry(entry.key()).or_insert(prior);
            }
        }
    }
    for state in deployments.values_mut() {
        for entry in &mut state.entries {
            if let Some(prior) = by_dest.get(&entry.key()) {
                entry.prior = Some((*prior).clone());
            }
        }
    }
}

/// A failed run's ONE compensating path (0025 §D): reconcile the
/// journal — the same code path a kill would take on the next run.
/// Compensation failure (quarantined entries fail closed) is logged,
/// not masked: the journal survives for the next run either way.
fn compensate(ctx: &Ctx) {
    match ctx
        .home_dir()
        .and_then(|h| store::journal::reconcile(h, &ctx.home))
    {
        Ok(lines) => {
            for line in lines {
                info!("{line}");
            }
        }
        Err(e) => {
            tracing::warn!("compensation reconcile failed (journal intact for next run): {e}")
        }
    }
}

/// Prune, manifest, and the generation-local environment profile —
/// everything fallible between deployment and the flip. Ok(None)
/// is the vacuous (satisfied) run.
#[allow(clippy::too_many_arguments)]
fn pre_flip(
    ctx: &Ctx,
    session: &crate::LifecycleSession,
    journal: &store::journal::JournalRun<'_>,
    prev_manifest: &Option<store::Generation>,
    deployments: &BTreeMap<String, store::DeploymentState>,
    next: store::GenerationId,
    deployment_reports: &[crate::report::StepReport],
    native: &crate::workspace::NativeProfiles,
) -> Result<Option<store::Generation>, ExecError> {
    // Prune-on-undeclare (0006 critique): destinations in the previous
    // manifest but gone now are removed — only if the file still matches
    // the recorded hash (user edits are never deleted). Every prune
    // mutation is journaled like a deploy (0025 §B).
    let active_directories = store::journal::directory::receipts(journal)?;
    if let Some(prev) = prev_manifest {
        prune_undeclared(prev, deployments, journal, &active_directories)?;
    }
    let mut directory_transitions = Vec::new();
    for receipt in active_directories {
        let leaves: Vec<_> = deployments
            .values()
            .flat_map(|state| &state.entries)
            .filter_map(|entry| {
                entry
                    .key()
                    .strip_prefix(&receipt.destination)
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .map(std::path::Path::to_owned)
            })
            .collect();
        if leaves.is_empty() {
            crate::deploy::directory_retirement::retire(journal, &receipt)?;
        } else {
            directory_transitions.push(store::journal::directory::finish(
                journal,
                &receipt.destination,
                leaves,
            )?);
        }
    }
    native.run_pre_flip_checks(ctx, session)?;
    // test-only kill switch: the prune→flip crash window's e2e (0025)
    crate::util::crash_hook("after-prune");

    // Satisfied = the deployment states are identical (the generation
    // number is not part of the comparison — 0008 §3) AND nothing
    // touched the filesystem. A run that repaired a destination —
    // an owned link swapped back to the store after drift, a stale
    // pre-store link replaced — changes disk state the manifest
    // cannot see; "already satisfied" and "I modified your
    // filesystem" must never both be true of one run (migration
    // report 0.18.1: a repaired symlink cut no generation, so
    // rollback could not undo it).
    let touched_disk = deployment_reports.iter().any(|r| {
        matches!(
            r.kind,
            crate::report::ReportKind::Installed | crate::report::ReportKind::Configured
        )
    });
    let migration_receipt = prev_manifest
        .as_ref()
        .and_then(|previous| previous.migration.clone());
    if prev_manifest.as_ref().is_some_and(|previous| {
        previous.deployments == *deployments
            && previous.directory_transitions == directory_transitions
            && previous.migration == migration_receipt
    }) && !touched_disk
    {
        return Ok(None);
    }
    let generation = store::Generation {
        number: next,
        deployments: deployments.clone(),
        directory_transitions,
        migration: migration_receipt,
    };
    // the generation publishes as ONE object (0027 §8): manifest and
    // profile stage under generations/.staging-<N> and rename into
    // place no-clobber — a failure here leaves nothing visible, let
    // alone activated
    let profile = crate::env::render_profile(&generation.deployments);
    store::generations::publish_generation(
        ctx.home_dir()?,
        &generation,
        profile.as_deref(),
        &ctx.home,
    )?;
    Ok(Some(generation))
}

/// Remove destinations the new manifest no longer declares, iff the
/// file on disk is still exactly what we deployed.
fn prune_undeclared(
    prev: &store::Generation,
    deployments: &BTreeMap<String, store::DeploymentState>,
    journal: &store::journal::JournalRun<'_>,
    directory_transitions: &[store::journal::directory::DirectoryTransition],
) -> Result<(), ExecError> {
    let home_dir = journal.home();
    let declared: BTreeSet<std::path::PathBuf> = deployments
        .values()
        .flat_map(|m| m.entries.iter().map(|e| e.key()))
        .collect();
    // merge blocks are owned per (deployment, dest) — several deployments may
    // hold blocks in one file, and a renamed deployment must not leave
    // its block behind as an unowned ghost (0026 §2). Non-merge
    // destinations are unique by E111, so a rename keeps the dest
    // deployed under the new deployment without a remove/redeploy churn.
    let declared_merge: BTreeSet<store::OwnershipKey> = deployments
        .iter()
        .flat_map(|(name, m)| {
            m.entries
                .iter()
                .filter(|e| e.ownership.policy() == gripsack_store::Ownership::Merge)
                .map(move |entry| entry.ownership_key(name))
        })
        .collect();
    for (name, state) in &prev.deployments {
        for entry in &state.entries {
            if directory_transitions
                .iter()
                .any(|receipt| receipt.destination == entry.key())
            {
                continue;
            }
            if store::journal::directory::is_directory(journal, &entry.key())? {
                continue;
            }
            // preserved drift was never written by gripsack — prune
            // never touches it (0029 §2; before this, the recorded
            // observed hash made the intact check pass and prune
            // DELETED the user's drifted file)
            if entry.preserved_drift {
                continue;
            }
            let declared_elsewhere = if entry.ownership.policy() == gripsack_store::Ownership::Merge
            {
                declared_merge.contains(&entry.ownership_key(name))
            } else {
                declared.contains(&entry.key())
            };
            if declared_elsewhere {
                continue;
            }
            // the ONE remove planner (0034): merge block-intactness,
            // the drift guard, and the journaled intent all live in
            // ops::plan — apply executes the same op rollback would
            let Some(op) = crate::ops::plan_remove_op(name, entry, &state.store_path, home_dir)?
            else {
                continue;
            };
            if matches!(op.kind(), crate::ops::OpKind::Preserved) {
                continue; // plan_remove_op already warned
            }
            crate::ops::execute_op(journal, op.as_executable()?)?;
            info!(
                "{} {}",
                if entry.prior.is_some() {
                    "restored prior"
                } else {
                    "pruned"
                },
                entry.to
            );
        }
    }
    Ok(())
}
