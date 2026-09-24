//! Typed reference admission (E126): every reference an output declares
//! — producer, runtime closure, package selections, prerequisites,
//! environments, schedules, hooks, publication checks, file sources —
//! must resolve to an admitted output of the expected kind, with spans
//! on both sites when the kind is wrong. Command bodies are admitted by
//! the sibling `context` module.

use super::context::{check_command, check_env_arg};
use super::names::Catalog;
use super::{expect, expect_artifact};
use crate::diagnostic::Diagnostic;
use crate::workspace::{Workspace, WorkspaceOutput, WorkspaceProducer, WorkspaceSource};

pub(super) fn check(workspace: &Workspace, catalog: &Catalog, diagnostics: &mut Vec<Diagnostic>) {
    for output in &workspace.outputs {
        match output {
            WorkspaceOutput::Recipe(recipe) => {
                for check_name in &recipe.checks {
                    expect(
                        catalog,
                        check_name,
                        &["check"],
                        &format!("recipe `{}` publication check", recipe.name),
                        &recipe.span,
                        diagnostics,
                    );
                }
                for step in &recipe.steps {
                    check_command(step, catalog, diagnostics);
                }
            }
            WorkspaceOutput::Package(package) => {
                match &package.producer {
                    WorkspaceProducer::Recipe { recipe } => expect(
                        catalog,
                        recipe,
                        &["recipe"],
                        &format!("package `{}` producer", package.name),
                        &package.span,
                        diagnostics,
                    ),
                    // A provider-backed package acquires directly — no
                    // catalog reference to check (0052 §2.2, A1-12).
                    WorkspaceProducer::Provider { .. } => {}
                }
                for runtime in &package.runtime {
                    expect(
                        catalog,
                        runtime,
                        &["package"],
                        &format!("package `{}` runtime closure", package.name),
                        &package.span,
                        diagnostics,
                    );
                }
            }
            WorkspaceOutput::Environment(environment) => {
                for package in &environment.packages {
                    expect(
                        catalog,
                        package,
                        &["package"],
                        &format!("environment `{}` package selection", environment.name),
                        &environment.span,
                        diagnostics,
                    );
                }
                for (variable, arg) in &environment.env {
                    check_env_arg(
                        arg,
                        &format!("environment `{}` variable `{variable}`", environment.name),
                        catalog,
                        &environment.span,
                        diagnostics,
                    );
                }
            }
            WorkspaceOutput::Task(task) => {
                check_command(&task.run, catalog, diagnostics);
                for dep in &task.deps {
                    expect(
                        catalog,
                        dep,
                        &["task"],
                        &format!("task `{}` prerequisite", task.name),
                        &task.span,
                        diagnostics,
                    );
                }
                if let Some(environment) = &task.environment {
                    expect(
                        catalog,
                        environment,
                        &["environment"],
                        &format!("task `{}` environment", task.name),
                        &task.span,
                        diagnostics,
                    );
                }
                for check_name in &task.checks {
                    expect(
                        catalog,
                        check_name,
                        &["check"],
                        &format!("task `{}` postcondition", task.name),
                        &task.span,
                        diagnostics,
                    );
                }
            }
            WorkspaceOutput::Schedule(schedule) => {
                expect(
                    catalog,
                    &schedule.task,
                    &["task"],
                    &format!("schedule `{}` task", schedule.name),
                    &schedule.span,
                    diagnostics,
                );
            }
            WorkspaceOutput::Check(check) => {
                check_command(&check.run, catalog, diagnostics);
            }
            WorkspaceOutput::Image(image) => {
                for package in &image.packages {
                    expect(
                        catalog,
                        package,
                        &["package"],
                        &format!("image `{}` package selection", image.name),
                        &image.span,
                        diagnostics,
                    );
                }
            }
            WorkspaceOutput::Profile(profile) => {
                if let Some(environment) = &profile.environment {
                    expect(
                        catalog,
                        environment,
                        &["environment"],
                        &format!("profile `{}` environment", profile.name),
                        &profile.span,
                        diagnostics,
                    );
                }
                for schedule in &profile.schedules {
                    expect(
                        catalog,
                        schedule,
                        &["schedule"],
                        &format!("profile `{}` schedule", profile.name),
                        &profile.span,
                        diagnostics,
                    );
                }
                for hook in &profile.hooks {
                    expect(
                        catalog,
                        hook,
                        &["hook"],
                        &format!("profile `{}` hook", profile.name),
                        &profile.span,
                        diagnostics,
                    );
                }
                for file in &profile.files {
                    if let Some(WorkspaceSource::ArtifactFile { output, .. }) = &file.source {
                        expect_artifact(
                            catalog,
                            output,
                            &format!("profile `{}` file source", profile.name),
                            &file.span,
                            diagnostics,
                        );
                    }
                }
            }
            WorkspaceOutput::Hook(hook) => {
                check_command(&hook.run, catalog, diagnostics);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::codes;
    use crate::sema::workspace::testutil::{PACKAGE, RECIPE, code_of, doc};

    #[test]
    fn invalid_references_are_rejected() {
        // producer recipe reference naming nothing
        let bad = PACKAGE.replace(r#""recipe": "build""#, r#""recipe": "nope""#);
        assert!(
            code_of(&doc(&format!("{RECIPE},{bad}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
        // producer recipe reference naming a non-recipe
        let bad = PACKAGE.replace(r#""recipe": "build""#, r#""recipe": "hello""#);
        assert!(
            code_of(&doc(&format!("{RECIPE},{bad}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
        // environment selecting a non-package
        let env = r#"{
            "kind": "environment", "name": "dev", "span": {"file": "grip.ts", "line": 4},
            "packages": ["build"], "target": {"os": "linux", "arch": "x86_64"}}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{env}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
    }

    #[test]
    fn task_schedule_profile_image_refs_are_typed() {
        let task = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 5},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 5},
                    "argv": [{"kind": "literal", "value": "true"}]},
            "deps": ["missing"]}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{task}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
        // schedule pointing at a non-task
        let schedule = r#"{
            "kind": "schedule", "name": "s", "span": {"file": "grip.ts", "line": 6},
            "task": "hello", "trigger": {"kind": "daily", "time": "09:30"}, "scope": "user"}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{schedule}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
        // profile hook reference naming a schedule
        let profile = r#"{
            "kind": "profile", "name": "p", "span": {"file": "grip.ts", "line": 7},
            "hooks": ["s"]}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{schedule},{profile}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
        // image selecting an unknown package
        let image = r#"{
            "kind": "image", "name": "img", "span": {"file": "grip.ts", "line": 8},
            "packages": ["ghost"], "target": {"os": "linux", "arch": "x86_64"}}"#;
        assert!(
            code_of(&doc(&format!("{RECIPE},{PACKAGE},{image}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
        // recipe publication check naming a task
        let recipe = RECIPE.replace(
            r#""execution": "native""#,
            r#""checks": ["t"], "execution": "native""#,
        );
        let task_ok = task.replace(r#""deps": ["missing"]"#, r#""deps": []"#);
        assert!(
            code_of(&doc(&format!("{recipe},{PACKAGE},{task_ok}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
    }
}
