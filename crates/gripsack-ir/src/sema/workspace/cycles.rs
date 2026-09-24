//! Dependency-cycle admission (E127) over the graph projection's
//! dependency roles — production, build input, runtime and task
//! prerequisite edges. Validation edges (publication checks,
//! postconditions, check subjects) and retention edges (schedule and
//! profile consumer wiring) are resolved and kind-checked in `refs`
//! but never feed the build closure, so a recipe gated by a check on
//! the package it produces is not a cycle. Edges that failed
//! resolution there (E126) are skipped here — one bad reference never
//! invents a phantom cycle. A cycle labels every member's declaration
//! span (0052 §2.1: both collision sources shown).

use super::graph::Projection;
use super::names::Catalog;
use crate::diagnostic::{Diagnostic, codes};
use std::collections::BTreeMap;

pub(super) fn check(projection: &Projection, catalog: &Catalog, diagnostics: &mut Vec<Diagnostic>) {
    // Adjacency over resolved dependency edges only.
    let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &projection.edges {
        if !edge.role.is_dependency() {
            continue;
        }
        let Some(target) = catalog.outputs.get(edge.to) else {
            continue;
        };
        if !edge.expected.contains(&target.kind()) {
            continue;
        }
        adjacency.entry(edge.from.name()).or_default().push(edge.to);
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit<'a>(
        node: &'a str,
        adjacency: &BTreeMap<&'a str, Vec<&'a str>>,
        catalog: &Catalog<'a>,
        marks: &mut BTreeMap<&'a str, Mark>,
        stack: &mut Vec<&'a str>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match marks.get(node) {
            Some(Mark::Done) => return,
            Some(Mark::Visiting) => {
                let start = stack.iter().position(|&n| n == node).unwrap_or(0);
                let mut cycle: Vec<&str> = stack[start..].to_vec();
                cycle.push(node);
                let mut diagnostic = Diagnostic::error(
                    codes::WORKSPACE_CYCLE,
                    format!(
                        "dependency cycle between workspace outputs: {}",
                        cycle.join(" -> ")
                    ),
                );
                for member in &cycle[..cycle.len() - 1] {
                    diagnostic = diagnostic.with_label(
                        Some(catalog.outputs[member].span().clone()),
                        format!("`{member}` declared here"),
                    );
                }
                diagnostics.push(diagnostic);
                return;
            }
            None => {}
        }
        marks.insert(node, Mark::Visiting);
        stack.push(node);
        if let Some(edges) = adjacency.get(node) {
            for &next in edges {
                visit(next, adjacency, catalog, marks, stack, diagnostics);
            }
        }
        stack.pop();
        marks.insert(node, Mark::Done);
    }
    let mut marks = BTreeMap::new();
    let mut stack = Vec::new();
    for name in catalog.outputs.keys().copied() {
        visit(
            name,
            &adjacency,
            catalog,
            &mut marks,
            &mut stack,
            diagnostics,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::codes;
    use crate::sema::workspace::testutil::{PACKAGE, RECIPE, code_of, doc};

    #[test]
    fn task_dependency_cycles_are_rejected() {
        let tasks = r#"
            {"kind": "task", "name": "a", "span": {"file": "grip.ts", "line": 2},
             "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 2},
                     "argv": [{"kind": "literal", "value": "a"}]},
             "deps": ["b"]},
            {"kind": "task", "name": "b", "span": {"file": "grip.ts", "line": 3},
             "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 3},
                     "argv": [{"kind": "literal", "value": "b"}]},
             "deps": ["a"]}"#;
        assert!(code_of(&doc(tasks)).contains(&codes::WORKSPACE_CYCLE.into()));
        let selfish = r#"
            {"kind": "task", "name": "a", "span": {"file": "grip.ts", "line": 2},
             "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 2},
                     "argv": [{"kind": "literal", "value": "a"}]},
             "deps": ["a"]}"#;
        assert!(code_of(&doc(selfish)).contains(&codes::WORKSPACE_CYCLE.into()));
    }

    #[test]
    fn production_cycles_label_every_member() {
        // the recipe builds with the very package it produces:
        // package → recipe (production) and recipe → package
        // (build-input tool reference) close a cycle
        let recipe = RECIPE.replace(
            r#""execution": "native""#,
            r#""steps": [{"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                          "argv": [{"kind": "package_command", "package": "hello", "command": "hello"}]}],
                "execution": "native""#,
        );
        let diagnostics = crate::check(&doc(&format!("{recipe},{PACKAGE}"))).unwrap_err();
        let cycle = diagnostics
            .iter()
            .find(|d| d.code == codes::WORKSPACE_CYCLE)
            .expect("E127 fired");
        assert!(cycle.message.contains("build") && cycle.message.contains("hello"));
        let lines: Vec<u32> = cycle
            .labels
            .iter()
            .filter_map(|l| l.span.as_ref().map(|s| s.line))
            .collect();
        assert_eq!(lines, vec![2, 3], "recipe and package spans labeled");
    }

    #[test]
    fn validation_edges_never_close_a_cycle() {
        // recipe gated by a check whose subject is the package the
        // recipe produces — a legitimate loop, not a dependency cycle
        let recipe = RECIPE.replace(
            r#""execution": "native""#,
            r#""checks": ["smoke"], "execution": "native""#,
        );
        let check = r#"{
            "kind": "check", "name": "smoke", "span": {"file": "grip.ts", "line": 5},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 5},
                    "argv": [{"kind": "literal", "value": "true"}]},
            "subject": "hello"}"#;
        crate::check(&doc(&format!("{recipe},{PACKAGE},{check}")))
            .expect("validation loop admitted");
    }
}
