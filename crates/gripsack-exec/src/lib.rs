//! Build ordering over the module graph (plan/0001 §4).
//!
//! Both runtime and build edges constrain order — an ephemeral build
//! dependency must exist before its dependent builds (0001 §3.1). The
//! executor consumes this order; parallel scheduling layers on top of it
//! without changing the semantics.

use gripsack_ir::Ir;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("dependency cycle involving: {}", .0.join(", "))]
    Cycle(Vec<String>),
}

/// Module names in dependency-first order. Deterministic: among modules
/// with equal priority, alphabetical — plans are diffable and stable.
pub fn build_order(ir: &Ir) -> Result<Vec<String>, PlanError> {
    // Kahn's algorithm with ordered sets for determinism.
    let mut indegree: BTreeMap<&str, usize> = ir.modules.keys().map(|k| (k.as_str(), 0)).collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, module) in &ir.modules {
        for dep in &module.depends {
            // Unknown deps are an IR-validation error, not a scheduling one.
            if let Some(n) = indegree.get_mut(name.as_str()) {
                *n += 1;
                dependents
                    .entry(dep.module.as_str())
                    .or_default()
                    .push(name);
            }
        }
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter(|(_, &n)| n == 0)
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
            .filter(|(_, &n)| n > 0)
            .map(|(&k, _)| k.to_owned())
            .collect();
        stuck.sort();
        return Err(PlanError::Cycle(stuck));
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::{Dependency, EdgeKind, Module, Source};
    use std::collections::BTreeMap;

    fn module_with_deps(deps: &[&str]) -> Module {
        Module {
            source: Some(Source::File { path: "/x".into() }),
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
            steps: None,
            span: None,
        }
    }

    fn ir(entries: &[(&str, &[&str])]) -> Ir {
        Ir {
            ir_version: 1,
            host: Default::default(),
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
        // `rust` is an ephemeral build-only dep of `helix` (0001 §3.1).
        let mut ir = ir(&[("helix", &["rust"]), ("rust", &[])]);
        ir.modules.get_mut("helix").unwrap().depends[0].edge = EdgeKind::Build;
        let order = build_order(&ir).unwrap();
        assert_eq!(order, vec!["rust", "helix"]);
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
