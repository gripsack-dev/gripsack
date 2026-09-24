//! Command context admission (E128): package commands appear only in
//! command positions — exec argv slots or the run_bash interpreter pin
//! — never in environment *values*, which are data (literal text or an
//! artifact reference), and the run_bash interpreter is always a pinned
//! `package_command` tool reference, never ambient host discovery
//! (0052 §2.2 CommandSpec boundaries, A1-03). The graph projection
//! collects these violations in the same walk that collects edges;
//! this module only judges them.

use super::graph::{ContextViolation, Projection};
use crate::diagnostic::{Diagnostic, codes};

pub(super) fn check(projection: &Projection, diagnostics: &mut Vec<Diagnostic>) {
    for violation in &projection.violations {
        match violation {
            ContextViolation::EnvCommand { relation, at } => diagnostics.push(
                Diagnostic::error(
                    codes::BAD_WORKSPACE_CONTEXT,
                    format!(
                        "{relation} cannot be a package_command reference; environment values are \
                         data (literal or artifact) — invoke package commands from exec argv or a \
                         run_bash interpreter pin"
                    ),
                )
                .with_label(Some((*at).clone()), "reference declared here"),
            ),
            ContextViolation::AmbientInterpreter { at } => diagnostics.push(
                Diagnostic::error(
                    codes::BAD_WORKSPACE_CONTEXT,
                    "run_bash interpreter must be a package_command reference pinning the \
                     tool through a declared package; a literal or artifact interpreter is \
                     ambient host discovery, which v4 never admits",
                )
                .with_label(Some((*at).clone()), "command declared here"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::sema::workspace::testutil::{PACKAGE, RECIPE, code_of, doc};
    use crate::{check, codes};

    #[test]
    fn command_references_and_contexts_are_admitted() {
        // valid: package_command in argv and as the run_bash interpreter pin
        let good = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 4},
            "run": {"kind": "run_bash", "span": {"file": "grip.ts", "line": 4},
                    "interpreter": {"kind": "package_command", "package": "hello", "command": "hello"},
                    "body": "echo ok"}}"#;
        check(&doc(&format!("{RECIPE},{PACKAGE},{good}"))).unwrap();

        // unknown package in package_command
        let bad_pkg = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 4},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                    "argv": [{"kind": "package_command", "package": "ghost", "command": "x"}]}}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{bad_pkg}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );

        // command the package does not export
        let bad_cmd = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 4},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                    "argv": [{"kind": "package_command", "package": "hello", "command": "nope"}]}}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{bad_cmd}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );

        // package_command as an environment value: E128
        let bad_ctx = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 4},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                    "argv": [{"kind": "literal", "value": "x"}],
                    "env": {"TOOL": {"kind": "package_command", "package": "hello", "command": "hello"}}}}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{bad_ctx}")))
                .contains(&codes::BAD_WORKSPACE_CONTEXT.into())
        );

        // literal interpreter: ambient bash discovery, E128
        let ambient = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 4},
            "run": {"kind": "run_bash", "span": {"file": "grip.ts", "line": 4},
                    "interpreter": {"kind": "literal", "value": "bash"},
                    "body": "echo ok"}}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{ambient}")))
                .contains(&codes::BAD_WORKSPACE_CONTEXT.into())
        );

        // unknown artifact argument
        let bad_art = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 4},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                    "argv": [{"kind": "artifact", "output": "ghost", "selector": "bin/x"}]}}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{bad_art}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
    }
}
