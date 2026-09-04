//! The executor (plan/0001 §4, 0007 §5): run a validated IR graph
//! against the store and deploy — one new generation per apply, or
//! "already satisfied" when nothing changed (0008 §3).
//!
//! ```text
//! apply.rs     the lifecycle: order → execute → compare → flip
//! module.rs    per-module phases: produce (A) → publish → deploy (B)
//! resolve.rs   fetch spec → concrete pin (lockfile wins)
//! deploy.rs    ownership modes + drift
//! verify.rs    smoke contracts
//! update.rs    the only lockfile mutator
//! ctx.rs       Ctx / Outcome / ExecError · report.rs — CLI reports
//! ```
//!
//! Execution is a ready-queue scheduler (schedule.rs): modules run as
//! their dependencies finish, N workers, named flock resources.

pub mod activate;
pub mod apply;
pub mod ctx;
pub mod deploy;
pub mod env;
pub mod expand;
pub mod facts;
pub mod frontend;
pub mod gc;
pub mod identity;
pub mod lockfile;
pub mod module;
pub mod report;
pub mod resolve;
pub mod rollback;
pub mod schedule;
pub mod template;
pub mod update;
pub mod util;
pub mod verify;
pub mod verify_store;

pub use apply::apply;
pub use ctx::{Ctx, ExecError, Outcome, ProgressCallback};
pub use env::render_env_file;
pub use frontend::{ensure_deno, ensure_ts_frontend};
pub use gc::{GcReport, gc, why_owns};
pub use report::{ApplyResult, ReportKind, StepReport, UpdateReport, UpdateStatus};
pub use rollback::rollback_generation;
pub use update::update;
pub use util::acquire_lifecycle_lock;

use gripsack_ir::Ir;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("dependency cycle involving: {0:?}")]
    Cycle(Vec<String>),
    #[error("module {0:?} depends on unknown module {1:?}")]
    UnknownDep(String, String),
}

/// Module names in dependency-first order. Deterministic: among modules
/// with equal priority, alphabetical — plans are diffable and stable.
pub fn build_order(ir: &Ir) -> Result<Vec<String>, PlanError> {
    // Kahn's algorithm with ordered sets for determinism.
    let mut indegree: BTreeMap<&str, usize> = ir.modules.keys().map(|k| (k.as_str(), 0)).collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, module) in &ir.modules {
        for dep in &module.depends {
            // sema (E101) catches this first; exec still refuses
            // honestly instead of misreporting a missing module as a
            // cycle (it would never enter `ready` and stick below)
            if !indegree.contains_key(dep.module.as_str()) {
                return Err(PlanError::UnknownDep(name.clone(), dep.module.clone()));
            }
            *indegree
                .get_mut(name.as_str())
                .expect("name is a graph key") += 1;
            dependents
                .entry(dep.module.as_str())
                .or_default()
                .push(name);
        }
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(&k, _)| k)
        .collect();
    let mut order = Vec::with_capacity(ir.modules.len());
    while let Some(&next) = ready.iter().next() {
        ready.remove(next);
        order.push(next.to_owned());
        for dependent in dependents.get(next).into_iter().flatten() {
            let n = indegree.get_mut(dependent).expect("dependent in graph");
            *n -= 1;
            if *n == 0 {
                ready.insert(dependent);
            }
        }
    }
    if order.len() != ir.modules.len() {
        let mut stuck: Vec<String> = indegree
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(&k, _)| k.to_owned())
            .collect();
        stuck.sort();
        return Err(PlanError::Cycle(stuck));
    }
    Ok(order)
}

/// Levelized view of the DAG for display: wave 0 = modules with no
/// dependencies, wave k = everything whose deps finished in waves < k.
pub fn waves(ir: &Ir) -> Result<Vec<Vec<String>>, PlanError> {
    let order = build_order(ir)?;
    let mut level: BTreeMap<&str, usize> = BTreeMap::new();
    for name in &order {
        let module = &ir.modules[name.as_str()];
        let l = module
            .depends
            .iter()
            .map(|d| level.get(d.module.as_str()).copied().unwrap_or(0) + 1)
            .max()
            .unwrap_or(0);
        level.insert(name.as_str(), l);
    }
    let mut waves: Vec<Vec<String>> = Vec::new();
    for (name, &l) in &level {
        while waves.len() <= l {
            waves.push(Vec::new());
        }
        waves[l].push(name.to_string());
    }
    Ok(waves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::{Dependency, EdgeKind, FetchSpec, Module};
    use std::collections::BTreeMap;

    fn module_with_deps(deps: &[&str]) -> Module {
        Module {
            fetch: Some(FetchSpec::File { path: "/x".into() }),
            build: Default::default(),
            install: vec![],
            config: vec![],
            depends: deps
                .iter()
                .map(|d| Dependency {
                    module: d.to_string(),
                    edge: EdgeKind::Runtime,
                    span: None,
                })
                .collect(),
            activate: vec![],
            env: vec![],
            steps: None,
            verify: None,
            retries: None,
            lint: None,
            span: None,
        }
    }

    fn ir(entries: &[(&str, &[&str])]) -> Ir {
        Ir {
            ir_version: 1,
            host: Default::default(),
            resources: vec![],
            modules: entries
                .iter()
                .map(|(name, deps)| (name.to_string(), module_with_deps(deps)))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn dependencies_come_first() {
        let ir = ir(&[("helix", &["git"]), ("git", &[]), ("zsh", &[])]);
        let order = build_order(&ir).unwrap();
        assert_eq!(order, vec!["git", "helix", "zsh"]);
    }

    #[test]
    fn diamond_is_deterministic() {
        let ir = ir(&[("a", &["b", "c"]), ("b", &["d"]), ("c", &["d"]), ("d", &[])]);
        let order = build_order(&ir).unwrap();
        assert_eq!(order, vec!["d", "b", "c", "a"]);
    }

    #[test]
    fn build_edges_constrain_order_too() {
        let mut ir = ir(&[("helix", &["rust"]), ("rust", &[])]);
        ir.modules.get_mut("helix").unwrap().depends[0].edge = EdgeKind::Build;
        let order = build_order(&ir).unwrap();
        assert_eq!(order, vec!["rust", "helix"]);
    }

    #[test]
    fn waves_levelize_the_dag() {
        let graph = ir(&[("a", &["b", "c"]), ("b", &["d"]), ("c", &["d"]), ("d", &[])]);
        let w = waves(&graph).unwrap();
        assert_eq!(
            w,
            vec![
                vec!["d".to_string()],
                vec!["b".to_string(), "c".to_string()],
                vec!["a".to_string()],
            ]
        );
        let graph = ir(&[("z", &["x"]), ("x", &[] as &[&str]), ("y", &[] as &[&str])]);
        let w = waves(&graph).unwrap();
        assert_eq!(w[0], vec!["x".to_string(), "y".to_string()]);
        assert_eq!(w[1], vec!["z".to_string()]);
    }

    #[test]
    fn cycles_are_named() {
        let ir = ir(&[("a", &["b"]), ("b", &["a"]), ("c", &[])]);
        match build_order(&ir) {
            Err(PlanError::Cycle(names)) => {
                assert!(names.contains(&"a".to_string()));
                assert!(names.contains(&"b".to_string()));
            }
            other => panic!("expected cycle error, got {other:?}"),
        }
    }
}
