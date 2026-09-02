//! `apply` — the lifecycle: order, execute, compare, flip (0001 §4).

use crate::ctx::Outcome;
use crate::ctx::{Ctx, ExecError};
use crate::env::render_env_file;
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
    let mut reports = Vec::new();
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
    let current_gen = store::current_generation(&ctx.home);
    let prev_manifest: Option<store::Generation> =
        current_gen.and_then(|n| store::read_manifest(&ctx.home, n).ok());
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
    if let Some((name, error, failed_state)) = outcome.failed {
        // Run-level rollback (0001 §9): the flip never happened, so
        // every destination this run touched goes back to the previous
        // generation's state — no half-applied deployment exists.
        let mut touched = outcome.modules;
        touched.insert(name, failed_state);
        crate::deploy::run_rollback(&touched, &prev_modules, &ctx.home);
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
    // pins are fetch outcomes — they must land before anything can
    // short-circuit later (the satisfied early-return used to skip
    // the write, so a mirror-swap re-fetch never refreshed its pin
    // and every later apply re-resolved)
    if lock_dirty {
        crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;
    }
    modules.extend(outcome.modules);

    // Prune-on-undeclare (0006 critique): destinations in the previous
    // manifest but gone now are removed — only if the file still matches
    // the recorded hash (user edits are never deleted).
    if let Some(prev) = &prev_manifest {
        prune_undeclared(prev, &modules, &ctx.home)?;
    }

    // Satisfied = the module states are identical (the generation
    // number is not part of the comparison — 0008 §3).
    let next = current_gen.unwrap_or(0) + 1;
    if prev_manifest.as_ref().map(|g| &g.modules) == Some(&modules) {
        return Ok(ApplyResult {
            outcome: Outcome::Satisfied {
                generation: current_gen,
            },
            reports,
        });
    }
    let generation = store::Generation {
        number: next,
        modules,
    };
    store::write_manifest(&ctx.home, &generation)?;
    // the exported-env profile renders BEFORE the flip: it names
    // store paths (already published), not the `current` link, so a
    // failure here leaves nothing activated — after the flip an
    // error would report apply-failed while the generation IS active
    render_env_file(&ctx.home, &generation.modules)?;
    store::flip(&ctx.home, next)?;
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

/// Remove destinations the new manifest no longer declares, iff the
/// file on disk is still exactly what we deployed.
fn prune_undeclared(
    prev: &store::Generation,
    modules: &BTreeMap<String, store::ModuleState>,
    home: &std::path::Path,
) -> Result<(), ExecError> {
    let declared: BTreeSet<&str> = modules
        .values()
        .flat_map(|m| m.entries.iter().map(|e| e.to.as_str()))
        .collect();
    for (name, state) in &prev.modules {
        for entry in &state.entries {
            if declared.contains(entry.to.as_str()) {
                continue;
            }
            let dest = gripsack_store::expand_home(&entry.to);
            if entry.mode == gripsack_ir::Ownership::Merge {
                // the file is foreign — prune removes only our block,
                // and only if the block content is still what we
                // deployed (a drifted block is the user's now)
                let existing = std::fs::read_to_string(&dest).unwrap_or_default();
                match crate::template::extract_block(&existing, name) {
                    Some(content)
                        if store::canonical_bytes_hash(content.as_bytes()) == entry.hash =>
                    {
                        let new = crate::template::remove_block(&existing, name)
                            .expect("block found above");
                        if new.trim().is_empty() {
                            std::fs::remove_file(&dest)?;
                        } else {
                            store::atomic_write(&dest, new.as_bytes())?;
                        }
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
            // not a deletion; the helper drift-guards and falls back
            // to removal when no prior was recorded
            if crate::deploy::remove_or_restore_prior(&dest, entry, name, home) {
                info!(
                    "{} {}",
                    if entry.prior.is_some() {
                        "restored prior"
                    } else {
                        "pruned"
                    },
                    entry.to
                );
            } else if dest.symlink_metadata().is_ok() {
                tracing::warn!("kept {} — modified since deploy", entry.to);
            }
        }
    }
    Ok(())
}
