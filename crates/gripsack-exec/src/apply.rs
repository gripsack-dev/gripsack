//! `apply` — the lifecycle: order, execute, compare, flip (0001 §4).

use crate::ctx::Outcome;
use crate::ctx::{Ctx, ExecError};
use crate::expand;
use crate::module::run_module;
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

    // The manifest starts from the current generation — a subset apply
    // replaces only the modules it touches (0001 §3.6).
    let current_gen = store::current_generation(&ctx.home);
    let mut modules: BTreeMap<String, store::ModuleState> = current_gen
        .and_then(|n| store::read_manifest(&ctx.home, n).ok())
        .map(|g| g.modules)
        .unwrap_or_default();

    for name in &order {
        let span = info_span!("module", name = name.as_str());
        let _entered = span.enter();
        let module = &ir.modules[name.as_str()];
        let steps = &steps_by_module[name.as_str()];
        let (state, module_reports, entry) = run_module(
            name,
            module,
            steps,
            ctx,
            modules.get(name.as_str()),
            lock.modules.get(name.as_str()),
        )?;
        reports.extend(module_reports);
        if let Some(entry) = entry
            && lock.modules.get(name.as_str()) != Some(&entry)
        {
            lock.modules.insert(name.clone(), entry);
            lock_dirty = true;
        }
        modules.insert(name.clone(), state);
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
