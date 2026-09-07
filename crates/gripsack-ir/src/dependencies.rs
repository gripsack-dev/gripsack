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

/// Host membership makes a module available to the graph. An incoming runtime
/// edge requires deployment; otherwise incoming build edges make it build-only.
/// Standalone modules deploy, and subset applies do not reinterpret the graph.
pub fn build_only_modules(modules: &BTreeMap<String, Module>) -> BTreeSet<String> {
    let mut build = BTreeSet::new();
    let mut runtime = BTreeSet::new();
    for dep in modules.values().flat_map(|m| &m.depends) {
        match dep.edge {
            EdgeKind::Build => {
                build.insert(dep.module.as_str());
            }
            EdgeKind::Runtime => {
                runtime.insert(dep.module.as_str());
            }
        }
    }
    build
        .difference(&runtime)
        .map(|name| (*name).to_owned())
        .collect()
}

/// Membership only: execution orders this set with the already-validated DAG.
/// Runtime edges are deliberately not followed. Unknown names and cycles are
/// diagnosed by the dependency checker and build_order, not hidden here.
pub fn build_closure_names<'a>(
    modules: &'a BTreeMap<String, Module>,
    name: &str,
) -> BTreeSet<&'a str> {
    let mut closure = BTreeSet::new();
    let mut pending = Vec::new();
    if let Some(module) = modules.get(name) {
        pending.push(module);
    }
    while let Some(module) = pending.pop() {
        for dep in &module.depends {
            if dep.edge == EdgeKind::Build
                && closure.insert(dep.module.as_str())
                && let Some(dependency) = modules.get(&dep.module)
            {
                pending.push(dependency);
            }
        }
    }
    closure.remove(name);
    closure
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
