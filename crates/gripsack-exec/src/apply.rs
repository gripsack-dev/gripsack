//! `apply` — the lifecycle: order, execute, compare, flip (0001 §4).

use crate::ctx::Outcome;
use crate::ctx::{Ctx, ExecError};
use crate::expand;
use crate::report::ApplyResult;
use gripsack_ir::Ir;
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use tracing::{info, info_span};

/// Apply the whole graph (or a subset) and activate a new generation.
/// The lifecycle — read current → build → flip — is one
/// read-modify-write, so the whole run holds `$GRIPSACK_HOME/
/// locks/apply.flock` (finding A): two concurrent applies serialize,
/// never lose a manifest update.
pub fn apply(ir: &Ir, ctx: &Ctx) -> Result<ApplyResult, ExecError> {
    let _lifecycle_lock = crate::util::acquire_lifecycle_lock(&ctx.home)?;
    // crash recovery (0019): a previous run killed between a deploy
    // mutation and the flip left uncommitted journal entries — the
    // filesystem sits between generations. Restore the priors before
    // deploying anything; the run then proceeds from a clean floor.
    let recovered = store::journal::reconcile(ctx.home_dir()?, &ctx.home)?;
    let mut reports = Vec::new();
    if !recovered.is_empty() {
        reports.push(crate::report::StepReport {
            module: "*".into(),
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
    let mut lock = match crate::lockfile::read(&ctx.repo, &ctx.host) {
        crate::lockfile::LockRead::Parsed(lock) => lock,
        crate::lockfile::LockRead::Missing => Default::default(),
        crate::lockfile::LockRead::Corrupt(why) => {
            // never silently re-pin from a corrupt lock: the file is
            // the tamper signal (lockfile.rs header). deleting it is
            // the user's deliberate re-pin.
            return Err(ExecError::Step {
                module: "*".into(),
                step: "lockfile".into(),
                detail: format!(
                    "{} is corrupt ({why}) — delete it to re-pin from scratch",
                    crate::lockfile::path(&ctx.repo, &ctx.host).display()
                ),
            });
        }
    };
    let mut lock_dirty = false;

    // The previous generation's state, for drift detection — distinct
    // from the manifest being built. Read once: prune and the
    // satisfied comparison below reuse it.
    let current_gen = store::current_generation(&ctx.home)?;
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
            store::read_manifest(&ctx.home, n).map_err(|e| ExecError::Step {
                module: "*".into(),
                step: "manifest".into(),
                detail: format!(
                    "current generation {n}'s manifest is unreadable ({e}) — refusing to apply"
                ),
            })?,
        ),
        None => None,
    };
    let prev_modules: BTreeMap<String, store::ModuleState> = prev_manifest
        .as_ref()
        .map(|g| g.modules.clone())
        .unwrap_or_default();
    // A subset apply starts from the current generation's manifest and
    // replaces only the modules it touches (0001 §3.6). A full apply
    // reconciles: modules absent from the IR are dropped from the
    // manifest — prune_undeclared then removes their destinations.
    let mut modules: BTreeMap<String, store::ModuleState> = if ctx.only.is_empty() {
        BTreeMap::new()
    } else {
        prev_modules.clone()
    };
    // The ready-queue scheduler (0007 §5): modules run as their
    // dependencies finish, N = cores, resources via flock. The flip
    // below stays the single barrier.
    let (order, missing) = scoped_order(ir, &ctx.only)?;
    if !missing.is_empty() {
        // a typo'd or host-gated name must not vanish — an apply that
        // "succeeded" while ignoring part of the request lies
        return Err(ExecError::Step {
            module: "*".into(),
            step: "scope".into(),
            detail: format!(
                "not in this host's graph: {} (the host entrypoint does not declare {} them)",
                missing.join(", "),
                if missing.len() == 1 { "it" } else { "all of" },
            ),
        });
    }
    let steps_by_module = expand::expand_all(&ir.modules);
    // physical destination uniqueness before anything mutates (0030)
    expand::check_physical_uniqueness(&ir.modules, &steps_by_module)?;
    // declare the generation this run builds BEFORE any mutation:
    // recovery compares the marker against `current` — a crash after
    // the flip but before journal cleanup must read as COMMITTED,
    // never restore priors the new generation owns (review 5.1)
    store::journal::begin_run(
        ctx.home_dir()?,
        current_gen,
        next_gen,
        store::journal::RunOp::Apply,
    )?;
    let outcome =
        crate::schedule::run_all(ir, &steps_by_module, &order, ctx, &prev_modules, &lock)?;
    // An empty result set must be a deliberate empty declaration,
    // never a scheduling artifact — prune can't tell them apart (B).
    if outcome.modules.is_empty() && !ir.modules.is_empty() && outcome.failed.is_none() {
        return Err(ExecError::Step {
            module: "*".into(),
            step: "schedule".into(),
            detail: "the scheduler produced zero module states from a non-empty graph".into(),
        });
    }
    if let Some((_name, error, _failed_state)) = outcome.failed {
        // Run-level rollback through the journal itself (0025 §D):
        // ONE compensating path — an ordinary failure reconciles
        // exactly like a kill would, restoring captured priors (which
        // honors post-crash user state better than replaying the
        // previous manifest did). No half-applied deployment exists.
        compensate(ctx);
        return Err(error);
    }
    for (name, module_reports) in outcome.reports {
        let span = info_span!("module", name = name.as_str());
        let _entered = span.enter();
        reports.extend(module_reports);
    }
    for (name, entry) in outcome.lock_entries {
        if lock.modules.get(&name) != Some(&entry) {
            lock.modules.insert(name, entry);
            lock_dirty = true;
        }
    }
    modules.extend(outcome.modules);
    if let Some(prev) = &prev_manifest {
        inherit_priors(prev, &mut modules);
    }
    let next = next_gen;

    // Everything between the scheduler and the flip is recoverable
    // through ONE compensating path (0025 §D): an ordinary error —
    // lockfile, prune, manifest, env render — reconciles exactly like
    // a kill would have. The flip itself is one atomic rename: an
    // error from it means nothing flipped, so it compensates too.
    match pre_flip(
        ctx,
        &prev_manifest,
        &modules,
        next,
        lock_dirty.then_some(&lock),
        &reports,
    ) {
        Ok(Some(_generation)) => {}
        // vacuous run: nothing journaled, nothing flipped — the
        // marker begin_run wrote must not linger
        Ok(None) => {
            store::journal::end_run(ctx.home_dir()?)?;
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
    if let Err(e) = store::flip(ctx.home_dir()?, &ctx.home, next) {
        compensate(ctx);
        return Err(e.into());
    }
    // the flip is the run's commit point: everything the journal
    // recorded is now owned by the new generation — the crash window
    // closes here
    if let Err(e) = store::journal::commit_run(ctx.home_dir()?) {
        // the flip already committed — this is cleanup-pending, not a
        // failed apply (0029 §13); the next run's reconcile finishes it
        reports.push(crate::report::StepReport {
            module: "*".into(),
            summary: format!(
                "generation {next} active; journal cleanup pending ({e}) — the next apply finishes it"
            ),
            kind: crate::report::ReportKind::Warned,
        });
    }
    reports.extend(crate::activate::run_post_link(&order, &steps_by_module));
    reports.extend(crate::activate::run_post_activate(&order, &steps_by_module));
    info!(generation = next, "activated");
    Ok(ApplyResult {
        outcome: Outcome::Applied { generation: next },
        reports,
    })
}

/// DAG order restricted to `only` + their transitive dependencies.
pub(crate) fn scoped_order(
    ir: &Ir,
    only: &[String],
) -> Result<(Vec<String>, Vec<String>), ExecError> {
    let order = crate::build_order(ir)?;
    if only.is_empty() {
        return Ok((order, Vec::new()));
    }
    let mut wanted: BTreeSet<&str> = only.iter().map(String::as_str).collect();
    let mut frontier: Vec<&str> = only.iter().map(String::as_str).collect();
    while let Some(name) = frontier.pop() {
        if let Some(m) = ir.modules.get(name) {
            for dep in &m.depends {
                if wanted.insert(dep.module.as_str()) {
                    frontier.push(dep.module.as_str());
                }
            }
        }
    }
    // names the caller asked for that this host's graph does not
    // declare — probe-gated modules and typos used to vanish
    // silently ("I asked for eight and got seven")
    let declared: BTreeSet<&str> = ir.modules.keys().map(String::as_str).collect();
    let missing: Vec<String> = only
        .iter()
        .filter(|n| !declared.contains(n.as_str()))
        .cloned()
        .collect();
    Ok((
        order
            .into_iter()
            .filter(|n| wanted.contains(n.as_str()))
            .collect(),
        missing,
    ))
}

/// Ownership lineage (0029 §1): an origin prior rides the whole
/// epoch — every generation carries it forward per destination until
/// a successful restore or prune ends the epoch. Keyed by
/// DESTINATION (a module rename transfers the origin to the new
/// module rather than losing it).
///
/// The inherited origin ALWAYS wins (0030 §H5): a second
/// `--take-over` captures the drifted bytes for the crash-recovery
/// journal, but the manifest keeps the epoch's FIRST origin —
/// undeclare must restore the pre-adoption state, not the last
/// absorbed drift.
fn inherit_priors(prev: &store::Generation, modules: &mut BTreeMap<String, store::ModuleState>) {
    let mut by_dest: std::collections::BTreeMap<&str, &store::Prior> = Default::default();
    for state in prev.modules.values() {
        for entry in &state.entries {
            if let Some(prior) = &entry.prior {
                by_dest.entry(entry.to.as_str()).or_insert(prior);
            }
        }
    }
    for state in modules.values_mut() {
        for entry in &mut state.entries {
            if let Some(prior) = by_dest.get(entry.to.as_str()) {
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

/// Lock write, prune, manifest, and the generation-local env profile —
/// everything fallible between the scheduler and the flip. Ok(None)
/// is the vacuous (satisfied) run.
fn pre_flip(
    ctx: &Ctx,
    prev_manifest: &Option<store::Generation>,
    modules: &BTreeMap<String, store::ModuleState>,
    next: u64,
    lock: Option<&crate::lockfile::Lockfile>,
    reports: &[crate::report::StepReport],
) -> Result<Option<store::Generation>, ExecError> {
    // pins are fetch outcomes — they must land before anything can
    // short-circuit later (the satisfied early-return used to skip
    // the write, so a mirror-swap re-fetch never refreshed its pin
    // and every later apply re-resolved)
    if let Some(lock) = lock {
        crate::lockfile::write(&ctx.repo, &ctx.host, lock)?;
    }

    // Prune-on-undeclare (0006 critique): destinations in the previous
    // manifest but gone now are removed — only if the file still matches
    // the recorded hash (user edits are never deleted). Every prune
    // mutation is journaled like a deploy (0025 §B).
    if let Some(prev) = prev_manifest {
        prune_undeclared(prev, modules, &ctx.home, ctx.home_dir()?)?;
    }
    // test-only kill switch: the prune→flip crash window's e2e (0025)
    crate::util::crash_hook("after-prune");

    // Satisfied = the module states are identical (the generation
    // number is not part of the comparison — 0008 §3) AND nothing
    // touched the filesystem. A run that repaired a destination —
    // an owned link swapped back to the store after drift, a stale
    // pre-store link replaced — changes disk state the manifest
    // cannot see; "already satisfied" and "I modified your
    // filesystem" must never both be true of one run (migration
    // report 0.18.1: a repaired symlink cut no generation, so
    // rollback could not undo it).
    let touched_disk = reports.iter().any(|r| {
        matches!(
            r.kind,
            crate::report::ReportKind::Installed | crate::report::ReportKind::Configured
        )
    });
    if prev_manifest.as_ref().map(|g| &g.modules) == Some(modules) && !touched_disk {
        return Ok(None);
    }
    let generation = store::Generation {
        number: next,
        modules: modules.clone(),
    };
    // the generation publishes as ONE object (0027 §8): manifest and
    // profile stage under generations/.staging-<N> and rename into
    // place no-clobber — a failure here leaves nothing visible, let
    // alone activated
    let profile = crate::env::render_profile(&generation.modules);
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
    modules: &BTreeMap<String, store::ModuleState>,
    home: &std::path::Path,
    home_dir: &gripsack_fs::Dir,
) -> Result<(), ExecError> {
    let declared: BTreeSet<&str> = modules
        .values()
        .flat_map(|m| m.entries.iter().map(|e| e.to.as_str()))
        .collect();
    // merge blocks are owned per (module, dest) — several modules may
    // hold blocks in one file, and a renamed module must not leave
    // its block behind as an unowned ghost (0026 §2). Non-merge
    // destinations are unique by E111, so a rename keeps the dest
    // deployed under the new module without a remove/redeploy churn.
    let declared_merge: BTreeSet<(&str, &str)> = modules
        .iter()
        .flat_map(|(name, m)| {
            m.entries
                .iter()
                .filter(|e| e.mode == gripsack_ir::Ownership::Merge)
                .map(|e| (name.as_str(), e.to.as_str()))
                .collect::<Vec<_>>()
        })
        .collect();
    for (name, state) in &prev.modules {
        for entry in &state.entries {
            // preserved drift was never written by gripsack — prune
            // never touches it (0029 §2; before this, the recorded
            // observed hash made the intact check pass and prune
            // DELETED the user's drifted file)
            if entry.preserved_drift {
                continue;
            }
            if entry.mode == gripsack_ir::Ownership::Merge {
                if declared_merge.contains(&(name.as_str(), entry.to.as_str())) {
                    continue;
                }
            } else if declared.contains(entry.to.as_str()) {
                continue;
            }
            let dest = gripsack_store::expand_home(&entry.to);
            if entry.mode == gripsack_ir::Ownership::Merge {
                // the file is foreign — prune removes only our block,
                // and only if the block content is still what we
                // deployed (a drifted block is the user's now)
                // NotFound-only-is-absent (0030 §H7): a permission or
                // I/O error is not an empty file
                let existing = match std::fs::read_to_string(&dest) {
                    Ok(t) => t,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(e) => return Err(e.into()),
                };
                match crate::template::extract_block(&existing, name) {
                    Some(content)
                        if store::canonical_bytes_hash(content.as_bytes()) == entry.hash =>
                    {
                        let new = crate::template::remove_block(&existing, name)
                            .expect("block found above");
                        // the intent is known before the mutation
                        // (0026 §6): REMOVED when the block was the
                        // whole file, the spliced content's hash else
                        let intended = if new.trim().is_empty() {
                            store::journal::REMOVED.to_string()
                        } else {
                            store::canonical_bytes_hash(new.as_bytes())
                        };
                        let (dest_dir, dest_name) = crate::deploy::dest_capability(&dest)?;
                        crate::deploy::journaled(
                            home_dir,
                            &dest_dir,
                            &dest_name,
                            &dest,
                            intended,
                            crate::deploy::Expect::Is(store::canonical_bytes_hash(
                                existing.as_bytes(),
                            )),
                            || {
                                if new.trim().is_empty() {
                                    dest_dir.remove_file(&dest_name)
                                } else {
                                    gripsack_fs::atomic_write(&dest_dir, &dest_name, new.as_bytes())
                                }
                            },
                        )?;
                        info!("pruned {} (block)", entry.to);
                    }
                    Some(_) => {
                        tracing::warn!("kept {} — block modified since deploy", entry.to)
                    }
                    None => {} // block already gone
                }
                continue;
            }
            // 0015 §4: an entry adopted with take-over (owned or
            // copy-like) gets its ORIGINAL file/symlink back on prune,
            // not a deletion; the drift guard runs FIRST (0026 §6) —
            // a kept destination is never journaled at all
            if !crate::deploy::intact_deployed(&dest, entry, &state.store_path) {
                if dest.symlink_metadata().is_ok() {
                    tracing::warn!("kept {} — modified since deploy", entry.to);
                }
                continue;
            }
            let intended = crate::deploy::prune_intent(entry, home)?;
            let (dest_dir, dest_name) = crate::deploy::dest_capability(&dest)?;
            let expected = match store::journal::live_identity(&dest_dir, &dest_name)? {
                Some(l) => crate::deploy::Expect::Is(l),
                None => crate::deploy::Expect::Absent,
            };
            crate::deploy::journaled(
                home_dir,
                &dest_dir,
                &dest_name,
                &dest,
                intended,
                expected,
                || {
                    // a failed removal is a transaction error now (0027
                    // §1), and journaled's postcondition verifies the
                    // landing regardless
                    crate::deploy::remove_or_restore_prior(
                        &dest_dir,
                        &dest_name,
                        entry,
                        name,
                        home,
                        &state.store_path,
                    )?;
                    Ok(())
                },
            )?;
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
