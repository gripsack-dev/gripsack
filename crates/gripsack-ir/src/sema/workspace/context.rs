//! Command and context admission: artifact references resolve (E126),
//! `package_command` invocations name an admitted package and one of
//! its exported commands (E126), package commands appear only in
//! command positions — never environment values (E128) — and the
//! run_bash interpreter is a pinned `package_command` tool reference,
//! never ambient host discovery (E128; 0052 §2.2, A1-03).

use super::expect_artifact;
use super::names::Catalog;
use crate::diagnostic::{Diagnostic, codes};
use crate::span::Span;
use crate::workspace::{WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath};

/// Command admission for one command body — exec or run_bash.
pub(super) fn check_command(
    command: &WorkspaceCommand,
    catalog: &Catalog,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match command {
        WorkspaceCommand::Exec {
            span,
            argv,
            env,
            cwd,
        } => {
            for arg in argv {
                check_command_arg(arg, catalog, span, diagnostics);
            }
            for (variable, arg) in env {
                check_env_arg(
                    arg,
                    &format!("command environment variable `{variable}`"),
                    catalog,
                    span,
                    diagnostics,
                );
            }
            if let Some(cwd) = cwd {
                check_path(cwd, catalog, span, diagnostics);
            }
        }
        WorkspaceCommand::RunBash {
            span,
            interpreter,
            env,
            cwd,
            ..
        } => {
            match interpreter {
                WorkspaceArg::PackageCommand { .. } => {
                    check_command_arg(interpreter, catalog, span, diagnostics);
                }
                _ => diagnostics.push(
                    Diagnostic::error(
                        codes::BAD_WORKSPACE_CONTEXT,
                        "run_bash interpreter must be a package_command reference pinning the \
                         tool through a declared package; a literal or artifact interpreter is \
                         ambient host discovery, which v4 never admits",
                    )
                    .with_label(Some(span.clone()), "command declared here"),
                ),
            }
            for (variable, arg) in env {
                check_env_arg(
                    arg,
                    &format!("command environment variable `{variable}`"),
                    catalog,
                    span,
                    diagnostics,
                );
            }
            if let Some(cwd) = cwd {
                check_path(cwd, catalog, span, diagnostics);
            }
        }
    }
}

/// An argument in a command position (exec argv slot or run_bash
/// interpreter pin) — the only places a package command may be invoked.
fn check_command_arg(
    arg: &WorkspaceArg,
    catalog: &Catalog,
    at: &Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match arg {
        WorkspaceArg::Literal { .. } => {}
        WorkspaceArg::Artifact { output, .. } => {
            expect_artifact(catalog, output, "command argument", at, diagnostics);
        }
        WorkspaceArg::PackageCommand { package, command } => {
            match catalog.outputs.get(package.as_str()) {
                None => diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!("package_command references unknown workspace output `{package}`"),
                    )
                    .with_label(Some(at.clone()), "reference declared here"),
                ),
                Some(target) if target.kind() != "package" => diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!(
                            "package_command must reference a package output; `{package}` is a {} output",
                            target.kind()
                        ),
                    )
                    .with_label(Some(at.clone()), "reference declared here")
                    .with_label(
                        Some(target.span().clone()),
                        format!("`{package}` declared here"),
                    ),
                ),
                Some(WorkspaceOutput::Package(target)) if !target.commands.contains_key(command) => {
                    diagnostics.push(
                        Diagnostic::error(
                            codes::UNKNOWN_WORKSPACE_REF,
                            format!(
                                "package_command references command `{command}`, which package `{package}` does not export"
                            ),
                        )
                        .with_label(Some(at.clone()), "reference declared here")
                        .with_label(
                            Some(target.span.clone()),
                            format!("`{package}` declared here"),
                        ),
                    );
                }
                Some(_) => {}
            }
        }
    }
}

/// An environment *value* is data — literal text or an artifact
/// reference. Invoking a package command here is E128: values never run
/// programs (0052 §2.2 CommandSpec boundaries).
pub(super) fn check_env_arg(
    arg: &WorkspaceArg,
    relation: &str,
    catalog: &Catalog,
    at: &Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match arg {
        WorkspaceArg::Literal { .. } => {}
        WorkspaceArg::Artifact { output, .. } => {
            expect_artifact(catalog, output, relation, at, diagnostics);
        }
        WorkspaceArg::PackageCommand { .. } => diagnostics.push(
            Diagnostic::error(
                codes::BAD_WORKSPACE_CONTEXT,
                format!(
                    "{relation} cannot be a package_command reference; environment values are \
                     data (literal or artifact) — invoke package commands from exec argv or a \
                     run_bash interpreter pin"
                ),
            )
            .with_label(Some(at.clone()), "reference declared here"),
        ),
    }
}

fn check_path(
    path: &WorkspacePath,
    catalog: &Catalog,
    at: &Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let WorkspacePath::Artifact { output, .. } = path {
        expect_artifact(
            catalog,
            output,
            "command working directory",
            at,
            diagnostics,
        );
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
