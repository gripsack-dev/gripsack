//! Task prerequisite cycle admission (E127): deps are verified within
//! one invocation, so a cycle could never start. Only edges to admitted
//! task outputs take part (dangling or wrong-kind deps are already
//! E126 in `refs`).

use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::{TaskOutput, Workspace, WorkspaceOutput};
use std::collections::BTreeMap;

pub(super) fn check(workspace: &Workspace, diagnostics: &mut Vec<Diagnostic>) {
    let tasks: BTreeMap<&str, &TaskOutput> = workspace
        .outputs
        .iter()
        .filter_map(|output| match output {
            WorkspaceOutput::Task(task) => Some((task.name.as_str(), task)),
            _ => None,
        })
        .collect();
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit<'a>(
        node: &'a str,
        tasks: &BTreeMap<&'a str, &'a TaskOutput>,
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
                diagnostics.push(
                    Diagnostic::error(
                        codes::WORKSPACE_CYCLE,
                        format!(
                            "cycle in workspace task dependencies: {}",
                            cycle.join(" -> ")
                        ),
                    )
                    .with_label(Some(tasks[node].span.clone()), "task declared here"),
                );
                return;
            }
            None => {}
        }
        marks.insert(node, Mark::Visiting);
        stack.push(node);
        // Copy the &'a TaskOutput out of the map so dep edges keep the
        // catalog lifetime instead of borrowing the map locally.
        let task: &'a TaskOutput = tasks[node];
        for dep in &task.deps {
            if tasks.contains_key(dep.as_str()) {
                visit(dep, tasks, marks, stack, diagnostics);
            }
        }
        stack.pop();
        marks.insert(node, Mark::Done);
    }
    let mut marks = BTreeMap::new();
    let mut stack = Vec::new();
    for name in tasks.keys().copied() {
        visit(name, &tasks, &mut marks, &mut stack, diagnostics);
    }
}

#[cfg(test)]
mod tests {
    use crate::codes;
    use crate::sema::workspace::testutil::{code_of, doc};

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
}
