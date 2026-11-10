//! `apply` — the lifecycle: order, execute, compare, flip (0001 §4).
//!
//! The kernel's apply machine (plan/0056 F2 step 7, [`machine`]) decides
//! every unit, every release, whether the run cuts a generation and when it
//! flips; this module is its host: it walks the workspace, executes what the
//! machine commands, and runs the parts the machine delegates (managed
//! blocks, the directory domain, activation).

mod dests;
mod machine;
pub(crate) use dests::same_destination;
pub(crate) use machine::{Commit, Machine, Verdict, agrees, refusal_error};

use crate::ctx::Outcome;
use crate::ctx::{Ctx, ExecError};
use crate::progress::Phase;
use crate::report::{ApplyResult, StepReport};
use gripsack_ir::Ir;
use gripsack_kernel::apply::{CommitInputs, OnDrift, Slot, UnitFacts, Want};
use gripsack_kernel::ownership::Recorded;
use gripsack_kernel::recover::{Failure, Intended, Route};
use gripsack_store as store;
use std::collections::BTreeMap;
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
    ctx.reporter().phase(Phase::Prepare);
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
    ctx.reporter().phase(Phase::Prepare);
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
    let reporter = ctx.reporter();
    // every report streams the moment it exists; the result keeps them too
    let report = |reports: &mut Vec<StepReport>, report: StepReport| {
        reporter.step(&report);
        reports.push(report);
    };
    let mut reports = Vec::new();
    if !recovered.is_empty() {
        report(
            &mut reports,
            StepReport {
                output: "*".into(),
                summary: format!(
                    "recovered {} destination(s) from an interrupted run",
                    recovered.len()
                ),
                tone: crate::report::Tone::Warned,
            },
        );
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
    let resumed = crate::activate::resume_activation(&session, reporter).map_err(|e| {
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
    // the kernel's apply machine decides every unit from here to the flip
    let mut machine = Machine::start().map_err(|refusal| refusal_error(refusal, "*", None))?;
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
    reporter.phase(Phase::Deploy);
    let execution = (|| {
        if let Some(previous) = &prev_manifest {
            for receipt in &previous.directory_transitions {
                store::journal::directory::bind_existing(&selection, receipt)?;
            }
        }
        native.deploy(
            ctx,
            &selection,
            &prev_deployments,
            migration.as_mut(),
            &mut machine,
        )
    })();
    let outcome = match execution {
        Ok(outcome) => outcome,
        Err(error) => {
            compensate(ctx);
            return Err(error);
        }
    };
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
        &native,
        &mut machine,
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
    // the machine names the selection to flip to: the reserved one, of an
    // admitted publication, after every unit settled (K6)
    match machine.published() {
        Ok(flip_to) if flip_to == *selection.identity() => {}
        Ok(_) | Err(_) => {
            compensate(ctx);
            return Err(refusal_error(
                gripsack_kernel::apply::Refusal::Protocol,
                "*",
                None,
            ));
        }
    }
    let committed = match store::flip(selection) {
        Ok(committed) => committed,
        Err(error) => {
            compensate(ctx);
            return Err(error.into());
        }
    };
    machine
        .flipped()
        .map_err(|refusal| refusal_error(refusal, "*", None))?;
    // test-only kill switch: the flip→adapters crash window's e2e
    crate::util::crash_hook("before-adapters");
    // the flip is the run's commit point: everything the journal
    // recorded is now owned by the new generation — the crash window
    // closes here
    if let Err(e) = store::journal::commit_run(committed) {
        // the flip already committed — this is cleanup-pending, not a
        // failed apply (0029 §13); the next run's reconcile finishes it
        report(
            &mut reports,
            StepReport {
                output: "*".into(),
                summary: format!(
                    "generation {next} active; journal cleanup pending ({e}) — the next apply finishes it"
                ),
                tone: crate::report::Tone::Warned,
            },
        );
    }
    if let Some(batch) = activation {
        reporter.phase(Phase::Activate);
        let labels = native.verification_labels();
        match crate::activate::run(batch, ctx.home_dir()?, &ctx.home, reporter, &labels) {
            Ok(outcomes) => reports.extend(outcomes),
            Err(error) => report(
                &mut reports,
                StepReport {
                    output: "*".into(),
                    summary: format!(
                        "generation {next} active; activation evidence retained ({error}) — inspect hooks before retrying"
                    ),
                    tone: crate::report::Tone::Warned,
                },
            ),
        }
    }
    // a hook's failure is recorded, never rolled back (0001 §3.8): the
    // machine finishes whatever activation returned
    machine
        .activated()
        .map_err(|refusal| refusal_error(refusal, "*", None))?;
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
    native: &crate::workspace::NativeProfiles,
    machine: &mut Machine,
) -> Result<Option<store::Generation>, ExecError> {
    let refused = |refusal| refusal_error(refusal, "*", None);
    // Prune-on-undeclare (0006 critique): destinations in the previous
    // manifest but gone now are removed — only if the file still matches
    // the recorded hash (user edits are never deleted). Every prune
    // mutation is journaled like a deploy (0025 §B).
    let active_directories = store::journal::directory::receipts(journal)?;
    if let Some(prev) = prev_manifest {
        prune_undeclared(prev, deployments, journal, &active_directories, machine)?;
    }
    // every unit settled: the directory domain's transitions (opaque until
    // plan/0056 step 9) run as the machine's one directory step
    machine.exhausted().map_err(refused)?;
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

    machine.directories_done().map_err(refused)?;
    // Satisfied = the deployment states are identical (the generation
    // number is not part of the comparison — 0008 §3) AND nothing
    // touched the filesystem. Whether the run touched the disk is the
    // kernel machine's own record (a unit landed, or a delegated unit
    // changed its file), never a report's presentation: "already
    // satisfied" and "I modified your filesystem" must never both be true
    // of one run (migration report 0.18.1: a repaired symlink cut no
    // generation, so rollback could not undo it).
    let migration_receipt = prev_manifest
        .as_ref()
        .and_then(|previous| previous.migration.clone());
    let unchanged = prev_manifest.as_ref().is_some_and(|previous| {
        previous.deployments == *deployments
            && previous.directory_transitions == directory_transitions
            && previous.migration == migration_receipt
    });
    // K6 (plan/0056 F2 steps 5 and 7): the machine decides whether this run
    // cuts a generation at all, and admits the publication only for the
    // allocated number, above the durable floor, not yet published, under a
    // reserved selection with a fresh transaction.
    let home_dir = ctx.home_dir()?;
    let commit = machine.commit(CommitInputs {
        previous: store::generations::current_selection_in(&ctx.home, home_dir)?,
        reserved: *journal.identity(),
        allocated: next,
        floor: store::generations::high_water(home_dir)?,
        unchanged,
        exists: store::generations::published(home_dir, next)?,
    });
    match commit {
        Ok(Commit::Vacuous) => return Ok(None),
        Ok(Commit::Publish(generation)) if generation == next => {}
        Ok(Commit::Publish(_)) => return Err(refused(gripsack_kernel::apply::Refusal::Protocol)),
        Err(gripsack_kernel::apply::Refusal::Unpublishable(why)) => {
            return Err(ExecError::Step {
                output: "*".into(),
                step: "generation".into(),
                detail: format!(
                    "generation {next} cannot be published ({why:?}) — refusing to activate it"
                ),
            });
        }
        Err(refusal) => return Err(refused(refusal)),
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

/// Release destinations the new manifest no longer deploys, iff the
/// destination is still exactly what gripsack deployed. The kernel's apply
/// machine decides each release: whether a deploy of this run claims the
/// destination (by the destination's identity, so a re-spelling the
/// filesystem judges the same name is not pruned under the new spelling —
/// plan/0056 FND-3), and whether the live object is gripsack's own write
/// (K1). Every prune mutation is journaled like a deploy (0025 §B).
fn prune_undeclared(
    prev: &store::Generation,
    deployments: &BTreeMap<String, store::DeploymentState>,
    journal: &store::journal::JournalRun<'_>,
    directory_transitions: &[store::journal::directory::DirectoryTransition],
    machine: &mut Machine,
) -> Result<(), ExecError> {
    let home_dir = journal.home();
    // the candidates: entries the new manifest no longer records under their
    // own spelling (a subset apply carries the other profiles' entries
    // forward without deploying them). Whole-file destinations are unique
    // (E111); merge blocks are owned per (deployment, destination), so a
    // renamed deployment does not leave its block behind (0026 §2).
    let recorded: std::collections::BTreeSet<std::path::PathBuf> = deployments
        .values()
        .flat_map(|state| state.entries.iter().map(|entry| entry.key()))
        .collect();
    let recorded_blocks: std::collections::BTreeSet<store::OwnershipKey> = deployments
        .iter()
        .flat_map(|(name, state)| {
            state
                .entries
                .iter()
                .filter(|entry| entry.ownership.policy() == gripsack_store::Ownership::Merge)
                .map(move |entry| entry.ownership_key(name))
        })
        .collect();
    for (name, state) in &prev.deployments {
        for entry in &state.entries {
            let still_recorded = if entry.ownership.policy() == gripsack_store::Ownership::Merge {
                recorded_blocks.contains(&entry.ownership_key(name))
            } else {
                recorded.contains(&entry.key())
            };
            if still_recorded {
                continue;
            }
            // the directory domain's destinations are its own (step 9)
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
            release(name, entry, &state.store_path, journal, home_dir, machine)?;
        }
    }
    Ok(())
}

/// One release unit through the machine.
fn release(
    name: &str,
    entry: &store::DeployedEntry,
    store_path: &std::path::Path,
    journal: &store::journal::JournalRun<'_>,
    home_dir: &gripsack_fs::Dir,
    machine: &mut Machine,
) -> Result<(), ExecError> {
    let fail = |detail: String| ExecError::Step {
        output: name.to_string(),
        step: "prune".into(),
        detail,
    };
    let refused = |refusal| refusal_error(refusal, name, Some(&entry.to));
    let dest = entry.key();
    let unit_dest = machine.dests.intern(&dest)?;
    let withheld = crate::ops::take_over_consent(false);
    if entry.ownership.policy() == gripsack_store::Ownership::Merge {
        // a managed block (K5): only an intact block is removed, by the
        // host's block decision; the machine decides whether a deploy of
        // this run still claims the block
        let owner = machine.owner(entry.merge_owner(name));
        let facts = UnitFacts {
            dest: unit_dest,
            slot: Slot::Block(owner),
            route: Route::Delegated,
            recorded: Recorded::Observed,
            want: Want::Release(Intended::Removed),
            consent: withheld,
            drift: OnDrift::Keep,
        };
        return match machine.unit(facts, None).map_err(refused)? {
            Verdict::Delegate => {
                let mutated = match crate::ops::plan_remove_op(name, entry, store_path, home_dir)? {
                    Some(op) if matches!(op.kind(), crate::ops::OpKind::Remove(_)) => {
                        crate::ops::execute_op(journal, op.as_executable()?)?;
                        info!("pruned block {}", entry.to);
                        true
                    }
                    _ => false,
                };
                machine.delegated(mutated).map_err(refused)
            }
            Verdict::Settled(_) => Ok(()),
            Verdict::Write { .. } => Err(refused(gripsack_kernel::apply::Refusal::Protocol)),
        };
    }
    let observation =
        observe_existing(&dest).map_err(|e| fail(format!("cannot inspect {}: {e}", entry.to)))?;
    let origin = store::journal::kernel_intended(
        &crate::deploy::restore::prune_intent(entry, home_dir).map_err(|e| fail(format!("{e}")))?,
    )?;
    let facts = UnitFacts {
        dest: unit_dest,
        slot: Slot::Whole,
        route: Route::Leaf,
        recorded: crate::ops::PreviousRecord { entry, store_path }.recorded(observation.as_ref()),
        want: Want::Release(origin),
        consent: withheld,
        drift: OnDrift::Keep,
    };
    let live = crate::ops::kernel_live(observation.as_ref());
    let verdict = machine.unit(facts, Some(live)).map_err(refused)?;
    if matches!(
        verdict,
        Verdict::Settled(gripsack_kernel::apply::Settled::Declared)
    ) {
        return Ok(());
    }
    // the ONE remove planner (0034) builds the op; it must be the one the
    // machine commanded (or none, or a kept destination, likewise)
    let op = crate::ops::plan_remove_op(name, entry, store_path, home_dir)?;
    let agreed = match (&op, &verdict) {
        (None, Verdict::Settled(gripsack_kernel::apply::Settled::Already)) => true,
        (Some(op), verdict) => agrees(op, verdict),
        _ => false,
    };
    if !agreed {
        return Err(fail(format!(
            "{}: the planner's removal does not match the kernel's decision ({verdict:?}) — a gripsack bug; nothing was written",
            entry.to
        )));
    }
    let (Some(op), Verdict::Write { .. }) = (op, verdict) else {
        return Ok(()); // already gone, or kept (plan_remove_op warned)
    };
    let executed = match crate::ops::execute_op(journal, op.as_executable()?) {
        Ok(executed) => executed,
        Err(error) => {
            machine.failed(Failure::Io(0));
            return Err(error);
        }
    };
    let landed = executed
        .landed
        .ok_or_else(|| fail(format!("{}: a commanded removal landed nothing", entry.to)))?;
    machine.landed(landed).map_err(refused)?;
    info!(
        "{} {}",
        if entry.prior.is_some() {
            "restored prior"
        } else {
            "pruned"
        },
        entry.to
    );
    Ok(())
}

/// A destination's one observation for a release: absent when it or its
/// parent is gone.
fn observe_existing(dest: &std::path::Path) -> std::io::Result<Option<crate::deploy::Observation>> {
    let parent = dest.parent().unwrap_or(std::path::Path::new("."));
    let directory = match gripsack_fs::open(parent) {
        Ok(directory) => directory,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let name = std::path::PathBuf::from(
        dest.file_name()
            .ok_or_else(|| std::io::Error::other("destination has no file name"))?,
    );
    crate::deploy::observe(&directory, &name)
}
