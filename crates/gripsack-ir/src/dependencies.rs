//! Dependency projections shared by semantic gates, preview and execution (0039).

use crate::{EdgeKind, Module};
use std::collections::{BTreeMap, BTreeSet};

/// Ordering constraints do not imply installation purposes or build exports.
/// `name` excludes self-qualified step references, which sema rejects.
pub fn ordering_dependencies<'a>(name: &str, module: &'a Module) -> BTreeSet<&'a str> {
    module
        .depends
        .iter()
        .map(|d| d.module.as_str())
        .chain(
            module
                .steps
                .iter()
                .flatten()
                .flat_map(|s| &s.needs)
                .filter_map(|need| need.split_once(':').map(|(target, _)| target))
                .filter(|target| *target != name),
        )
        .collect()
}
/// The name-indexed view the verified kernels (0047) consume: every
/// module key plus every referenced build target — unknown names are
/// sink nodes, the pre-0047 semantics exactly. Edges carry indices
/// into the table.
fn index_view<'a>(modules: &'a BTreeMap<String, Module>) -> (Vec<&'a str>, Vec<Vec<usize>>) {
    let mut names: Vec<&'a str> = modules.keys().map(String::as_str).collect();
    let mut index: BTreeMap<&'a str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (*name, i))
        .collect();
    for module in modules.values() {
        for dep in &module.depends {
            if dep.edge == EdgeKind::Build && !index.contains_key(dep.module.as_str()) {
                index.insert(dep.module.as_str(), names.len());
                names.push(dep.module.as_str());
            }
        }
    }
    // sinks (referenced but not declared) carry no out-edges; the
    // kernel's admission contract wants one edge list per name
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); names.len()];
    for (i, module) in modules.values().enumerate() {
        edges[i] = module
            .depends
            .iter()
            .filter(|dep| dep.edge == EdgeKind::Build)
            .map(|dep| index[dep.module.as_str()])
            .collect();
    }
    (names, edges)
}

/// Host membership makes a module available to the graph. An incoming runtime
/// edge requires deployment; otherwise incoming build edges make it build-only.
/// Standalone modules deploy, and subset applies do not reinterpret the graph.
/// The set arithmetic is the verified kernel (0047).
pub fn build_only_modules(modules: &BTreeMap<String, Module>) -> BTreeSet<String> {
    let mut names: Vec<&str> = modules.keys().map(String::as_str).collect();
    let mut index: BTreeMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (*name, i))
        .collect();
    let mut build_target = vec![false; names.len()];
    let mut runtime_target = vec![false; names.len()];
    for dep in modules.values().flat_map(|m| &m.depends) {
        // an unknown referenced name is an edge target too (the
        // pre-0047 set could contain it) — the table grows to cover it
        let idx = match index.get(dep.module.as_str()) {
            Some(i) => *i,
            None => {
                let i = names.len();
                names.push(dep.module.as_str());
                index.insert(dep.module.as_str(), i);
                build_target.push(false);
                runtime_target.push(false);
                i
            }
        };
        match dep.edge {
            EdgeKind::Build => build_target[idx] = true,
            EdgeKind::Runtime => runtime_target[idx] = true,
        }
    }
    gripsack_policy::graph::build_only_members(&build_target, &runtime_target)
        .into_iter()
        .map(|i| names[i].to_owned())
        .collect()
}

/// Membership only: execution orders this set with the already-validated DAG.
/// Runtime edges are deliberately not followed. Unknown names and cycles are
/// diagnosed by the dependency checker and build_order, not hidden here.
/// The transitive walk is the verified kernel (0047).
pub fn build_closure_names<'a>(
    modules: &'a BTreeMap<String, Module>,
    name: &str,
) -> BTreeSet<&'a str> {
    let (names, edges) = index_view(modules);
    let Some(root) = names.iter().position(|n| *n == name) else {
        return BTreeSet::new();
    };
    gripsack_policy::graph::build_closure(names.len(), &edges, root)
        .into_iter()
        .map(|i| names[i])
        .collect()
}

/// Module names are ASCII (E116). The prefix makes even digit-leading names
/// valid identifiers; collisions after normalization are rejected by E123.
pub fn build_dep_var(name: &str) -> String {
    let mut var = String::with_capacity(9 + name.len());
    var.push_str("GRIP_DEP_");
    for c in name.chars() {
        var.push(if c.is_ascii_alphanumeric() {
            c.to_ascii_uppercase()
        } else {
            '_'
        });
    }
    var
}
