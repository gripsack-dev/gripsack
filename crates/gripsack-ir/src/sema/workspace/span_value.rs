//! Provenance and combined-value admission: v4 spans are mandatory AND
//! well-formed (E129), and values the grammar admits only in combination
//! — a non-empty catalog, non-empty output names, HH:MM calendar clocks,
//! a file origin for non-literal content — are rejected with E130.

use crate::diagnostic::{Diagnostic, codes};
use crate::span::Span;
use crate::workspace::{
    Workspace, WorkspaceCalendar, WorkspaceCommand, WorkspaceContent, WorkspaceOutput,
    WorkspaceProducer,
};

pub(super) fn check(workspace: &Workspace, diagnostics: &mut Vec<Diagnostic>) {
    check_spans(workspace, diagnostics);
    check_values(workspace, diagnostics);
}

/// E129 — a span with an empty file or a line/column below 1 is a
/// frontend defect, surfaced like any other admission error.
fn check_spans(workspace: &Workspace, diagnostics: &mut Vec<Diagnostic>) {
    let mut spans: Vec<&Span> = vec![&workspace.span];
    for output in &workspace.outputs {
        spans.push(output.span());
        match output {
            WorkspaceOutput::Recipe(recipe) => {
                spans.push(&recipe.source.span);
                spans.extend(recipe.steps.iter().map(WorkspaceCommand::span));
            }
            WorkspaceOutput::Package(package) => {
                if let WorkspaceProducer::Provider { provider } = &package.producer {
                    spans.push(&provider.span);
                }
            }
            WorkspaceOutput::Task(task) => spans.push(task.run.span()),
            WorkspaceOutput::Check(check) => spans.push(check.run.span()),
            WorkspaceOutput::Hook(hook) => spans.push(hook.run.span()),
            WorkspaceOutput::Profile(profile) => {
                spans.extend(profile.files.iter().map(|file| &file.span));
            }
            _ => {}
        }
    }
    for span in spans {
        if span.file.is_empty() {
            diagnostics.push(Diagnostic::error(
                codes::BAD_WORKSPACE_SPAN,
                format!("workspace span {span} has an empty file; v4 provenance requires the declaring source file"),
            ));
        }
        if span.line == 0 {
            diagnostics.push(Diagnostic::error(
                codes::BAD_WORKSPACE_SPAN,
                format!("workspace span {span} has line 0; v4 provenance lines start at 1"),
            ));
        }
        if span.col == Some(0) {
            diagnostics.push(Diagnostic::error(
                codes::BAD_WORKSPACE_SPAN,
                format!("workspace span {span} has column 0; v4 provenance columns start at 1"),
            ));
        }
    }
}

/// E130 — values the grammar admits only in combination.
fn check_values(workspace: &Workspace, diagnostics: &mut Vec<Diagnostic>) {
    if workspace.outputs.is_empty() {
        diagnostics.push(
            Diagnostic::error(
                codes::INVALID_WORKSPACE_VALUE,
                "workspace declares no outputs; the catalog needs at least one named output",
            )
            .with_label(Some(workspace.span.clone()), "workspace declared here"),
        );
    }
    for output in &workspace.outputs {
        if output.name().is_empty() {
            diagnostics.push(
                Diagnostic::error(
                    codes::INVALID_WORKSPACE_VALUE,
                    format!(
                        "a {} output is declared with an empty name; output names are the catalog namespace",
                        output.kind()
                    ),
                )
                .with_label(Some(output.span().clone()), "output declared here"),
            );
        }
        match output {
            WorkspaceOutput::Schedule(schedule) => {
                let time = match &schedule.trigger {
                    WorkspaceCalendar::Daily { time } => time,
                    WorkspaceCalendar::Weekly { time, .. } => time,
                };
                if !valid_local_time(time) {
                    diagnostics.push(
                        Diagnostic::error(
                            codes::INVALID_WORKSPACE_VALUE,
                            format!(
                                "schedule `{}` trigger time `{time}` is not HH:MM local time; named timezones, intervals and cron are not admitted grammar",
                                schedule.name
                            ),
                        )
                        .with_label(Some(schedule.span.clone()), "schedule declared here"),
                    );
                }
            }
            WorkspaceOutput::Profile(profile) => {
                for file in &profile.files {
                    let needs_source = !matches!(file.content, WorkspaceContent::Literal { .. });
                    if needs_source && file.source.is_none() {
                        diagnostics.push(
                            Diagnostic::error(
                                codes::INVALID_WORKSPACE_VALUE,
                                format!(
                                    "profile `{}` file has {} content without a source; only literal content may omit `source`",
                                    profile.name,
                                    match &file.content {
                                        WorkspaceContent::Identity => "identity",
                                        WorkspaceContent::Template { .. } => "template",
                                        WorkspaceContent::Literal { .. } => unreachable!(),
                                    }
                                ),
                            )
                            .with_label(Some(file.span.clone()), "file declared here"),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// HH:MM local time, 00–23 : 00–59 — the schema's
/// `^([01][0-9]|2[0-3]):[0-5][0-9]$` pattern as a parser check so
/// admission, not just frontend authoring, enforces it.
fn valid_local_time(time: &str) -> bool {
    let bytes = time.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return false;
    }
    let (hour, minute) = (&time[..2], &time[3..]);
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    digits(hour)
        && digits(minute)
        && hour.parse::<u32>().is_ok_and(|h| h <= 23)
        && minute.parse::<u32>().is_ok_and(|m| m <= 59)
}

#[cfg(test)]
mod tests {
    use crate::sema::workspace::testutil::{PACKAGE, RECIPE, code_of, doc};
    use crate::{check, codes};

    #[test]
    fn degenerate_spans_and_values_are_rejected() {
        let empty_file = RECIPE.replace(
            r#""file": "grip.ts", "line": 2}"#,
            r#""file": "", "line": 2}"#,
        );
        assert!(
            code_of(&doc(&format!("{empty_file},{PACKAGE}")))
                .contains(&codes::BAD_WORKSPACE_SPAN.into())
        );
        let zero_line = RECIPE.replace(
            r#""file": "grip.ts", "line": 2}"#,
            r#""file": "grip.ts", "line": 0}"#,
        );
        assert!(
            code_of(&doc(&format!("{zero_line},{PACKAGE}")))
                .contains(&codes::BAD_WORKSPACE_SPAN.into())
        );
        // empty catalog
        let empty = r#"{"ir_version": 4, "host": {"os": "linux", "arch": "x86_64"}, "workspace": {"span": {"file": "grip.ts", "line": 1}, "outputs": []}}"#;
        assert!(code_of(empty).contains(&codes::INVALID_WORKSPACE_VALUE.into()));
        // bad calendar clock
        let schedule = r#"{
            "kind": "schedule", "name": "s", "span": {"file": "grip.ts", "line": 6},
            "task": "t", "trigger": {"kind": "daily", "time": "25:00"}, "scope": "user"}"#;
        let task = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 5},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 5},
                    "argv": [{"kind": "literal", "value": "true"}]}}"#;
        assert!(
            code_of(&doc(&format!("{task},{schedule}")))
                .contains(&codes::INVALID_WORKSPACE_VALUE.into())
        );
    }

    #[test]
    fn file_content_requires_source_unless_literal() {
        let identity_no_source = r#"{
            "kind": "profile", "name": "p", "span": {"file": "grip.ts", "line": 2},
            "files": [{
                "span": {"file": "grip.ts", "line": 3},
                "content": {"kind": "identity"},
                "destination": {"kind": "symlink", "path": "~/.vimrc"}}]}"#;
        assert!(code_of(&doc(identity_no_source)).contains(&codes::INVALID_WORKSPACE_VALUE.into()));
        let template_no_source = r#"{
            "kind": "profile", "name": "p", "span": {"file": "grip.ts", "line": 2},
            "files": [{
                "span": {"file": "grip.ts", "line": 3},
                "content": {"kind": "template", "template": "x", "variables": {}},
                "destination": {"kind": "symlink", "path": "~/.vimrc"}}]}"#;
        assert!(code_of(&doc(template_no_source)).contains(&codes::INVALID_WORKSPACE_VALUE.into()));
        // identity with a source admits
        let identity_sourced = r#"{
            "kind": "profile", "name": "p", "span": {"file": "grip.ts", "line": 2},
            "files": [{
                "span": {"file": "grip.ts", "line": 3},
                "source": {"kind": "repo_file", "path": "vim/vimrc"},
                "content": {"kind": "identity"},
                "destination": {"kind": "symlink", "path": "~/.vimrc"}}]}"#;
        check(&doc(identity_sourced)).unwrap();
    }
}
