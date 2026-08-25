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
pub fn apply(ir: &Ir, ctx: &Ctx) -> Result<ApplyResult, ExecError> {
    let order = scoped_order(ir, &ctx.only)?;
    let steps_by_module = expand::expand_all(&ir.modules);
    let mut reports = Vec::new();
    let mut lock = crate::lockfile::read(&ctx.repo, &ctx.host).unwrap_or_default();
    let mut lock_dirty = false;

    // The previous generation's state, for drift detection — distinct
    // from the manifest being built (see below).
    let current_gen = store::current_generation(&ctx.home);
    let prev_modules: BTreeMap<String, store::ModuleState> = current_gen
        .and_then(|n| store::read_manifest(&ctx.home, n).ok())
        .map(|g| g.modules)
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

    // Prune-on-undeclare (0006 critique): destinations in the previous
    // manifest but gone now are removed — only if the file still matches
    // the recorded hash (user edits are never deleted).
    if let Some(n) = current_gen
        && let Ok(prev) = store::read_manifest(&ctx.home, n)
    {
        prune_undeclared(&prev, &modules, &ctx.home)?;
    }

    // Satisfied = the module states are identical (the generation
    // number is not part of the comparison — 0008 §3).
    let next = current_gen.unwrap_or(0) + 1;
    if current_gen
        .and_then(|n| store::read_manifest(&ctx.home, n).ok())
        .map(|g| g.modules)
        .as_ref()
        == Some(&modules)
    {
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
    store::flip(&ctx.home, next)?;
    render_env_file(&ctx.home, &generation.modules)?;
    reports.extend(crate::activate::run_post_activate(&order, &ir.modules));
    if lock_dirty {
        crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;
    }
    info!(generation = next, "activated");
    Ok(ApplyResult {
        outcome: Outcome::Applied { generation: next },
        reports,
    })
}

/// DAG order restricted to `only` + their transitive dependencies.
pub(crate) fn scoped_order(ir: &Ir, only: &[String]) -> Result<Vec<String>, ExecError> {
    let order = crate::build_order(ir)?;
    if only.is_empty() {
        return Ok(order);
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
    Ok(order
        .into_iter()
        .filter(|n| wanted.contains(n.as_str()))
        .collect())
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
    for state in prev.modules.values() {
        for entry in &state.entries {
            if declared.contains(entry.to.as_str()) {
                continue;
            }
            let dest = crate::deploy::expand_home(&entry.to);
            if entry.mode == gripsack_ir::Ownership::Owned {
                // Owned destinations are symlinks into the store; the
                // recorded hash is the *source content*, so the hash
                // check below can never match (it would hash the link
                // target string). "Unmodified" for owned means: still
                // our symlink — same test deploy uses (0009).
                let ours = std::fs::read_link(&dest)
                    .map(|t| t.starts_with(home))
                    .unwrap_or(false);
                if ours {
                    std::fs::remove_file(&dest)?;
                    info!("pruned {}", entry.to);
                } else if dest.symlink_metadata().is_ok() {
                    tracing::warn!("kept {} — replaced since deploy", entry.to);
                }
                continue;
            }
            let Ok(current) = store::canonical_file_hash(&dest) else {
                continue; // already gone
            };
            if current == entry.hash {
                std::fs::remove_file(&dest)?;
                info!("pruned {}", entry.to);
            } else {
                tracing::warn!("kept {} — modified since deploy", entry.to);
            }
        }
    }
    Ok(())
}
