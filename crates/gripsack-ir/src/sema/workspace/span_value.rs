//! Provenance and combined-value admission: workspace spans are mandatory
//! and well-formed (E129). Values admitted only in combination — nonempty
//! catalogs/names, calendar clocks, file origins and literal Bash bodies
//! — are checked before an E124 execution-capability decision (E130).

use crate::diagnostic::{Diagnostic, codes};
use crate::span::Span;
use crate::workspace::{
    PackageLayout, Workspace, WorkspaceCalendar, WorkspaceCommand, WorkspaceContent,
    WorkspaceDestination, WorkspaceOutput, WorkspacePlatform, WorkspaceProducer, WorkspaceSource,
};

pub(super) fn check(workspace: &Workspace, diagnostics: &mut Vec<Diagnostic>) {
    check_spans(workspace, diagnostics);
    check_values(workspace, diagnostics);
}

/// E129 — a span with an empty file or a line/column below 1 is a
/// frontend defect, surfaced like any other admission error.
fn check_spans(workspace: &Workspace, diagnostics: &mut Vec<Diagnostic>) {
    // Walk borrowed declarations directly: a valid workspace needs no
    // temporary span vector just to check its provenance.
    let mut check = |span: &Span| {
        if span.file.is_empty() {
            diagnostics.push(
                Diagnostic::error(
                    codes::BAD_WORKSPACE_SPAN,
                    format!("workspace span {span} has an empty file; provenance requires the declaring source file"),
                )
                .with_label(Some(span.clone()), "invalid source span declared here"),
            );
        }
        if span.line == 0 {
            diagnostics.push(
                Diagnostic::error(
                    codes::BAD_WORKSPACE_SPAN,
                    format!("workspace span {span} has line 0; source lines start at 1"),
                )
                .with_label(Some(span.clone()), "invalid source span declared here"),
            );
        }
        if span.col == Some(0) {
            diagnostics.push(
                Diagnostic::error(
                    codes::BAD_WORKSPACE_SPAN,
                    format!("workspace span {span} has column 0; source columns start at 1"),
                )
                .with_label(Some(span.clone()), "invalid source span declared here"),
            );
        }
    };
    check(&workspace.span);
    for output in &workspace.outputs {
        check(output.span());
        match output {
            WorkspaceOutput::Recipe(recipe) => {
                check(&recipe.source.span);
                for step in &recipe.steps {
                    check(step.span());
                }
            }
            WorkspaceOutput::Package(package) => {
                if let WorkspaceProducer::Provider { provider } = &package.producer {
                    check(&provider.span);
                }
            }
            WorkspaceOutput::Task(task) => check(task.run.span()),
            WorkspaceOutput::Check(check_output) => check(check_output.run.span()),
            WorkspaceOutput::Hook(hook) => check(hook.run.span()),
            WorkspaceOutput::Profile(profile) => {
                for file in &profile.files {
                    check(&file.span);
                }
            }
            _ => {}
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
        let target: Option<&WorkspacePlatform> = match output {
            WorkspaceOutput::Recipe(recipe) => Some(&recipe.target),
            WorkspaceOutput::Package(package) => Some(&package.target),
            WorkspaceOutput::Environment(environment) => Some(&environment.target),
            WorkspaceOutput::Image(image) => Some(&image.target),
            _ => None,
        };
        if target.is_some_and(|target| !target.valid_abi()) {
            diagnostics.push(
                Diagnostic::error(
                    codes::INVALID_WORKSPACE_VALUE,
                    format!(
                        "{} `{}` declares an ABI incompatible with its target OS; Linux admits gnu/musl and macOS admits darwin",
                        output.kind(), output.name()
                    ),
                )
                .with_label(Some(output.span().clone()), "target declared here"),
            );
        }
        let prefix = match output {
            WorkspaceOutput::Package(package) => match &package.layout {
                PackageLayout::FixedPrefix { prefix } => Some(prefix),
                PackageLayout::Relocatable => None,
            },
            WorkspaceOutput::Environment(environment) => environment.prefix.as_ref(),
            _ => None,
        };
        if let Some(prefix) = prefix
            && !prefix.is_safe()
        {
            diagnostics.push(
                    Diagnostic::error(
                        codes::INVALID_WORKSPACE_VALUE,
                        format!(
                            "{} `{}` has an unsafe install prefix {:?}; use a normalized absolute POSIX path below /",
                            output.kind(), output.name(), prefix.as_str()
                        ),
                    )
                    .with_label(Some(output.span().clone()), "prefix declared here"),
                );
        }
        match output {
            WorkspaceOutput::Recipe(recipe) => {
                for command in &recipe.steps {
                    check_bash_body(command, diagnostics);
                }
            }
            WorkspaceOutput::Task(task) => check_bash_body(&task.run, diagnostics),
            WorkspaceOutput::Check(check) => check_bash_body(&check.run, diagnostics),
            WorkspaceOutput::Hook(hook) => check_bash_body(&hook.run, diagnostics),
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
                    if let Some(WorkspaceSource::RepoFile { path }) = &file.source
                        && !admissible_repo_file(path)
                    {
                        diagnostics.push(
                            Diagnostic::error(
                                codes::INVALID_WORKSPACE_VALUE,
                                format!(
                                    "profile `{}` file source {:?} must be a normalized repository-relative POSIX file path — no root, backslash, NUL, empty, \".\" or \"..\" segments",
                                    profile.name, path
                                ),
                            )
                            .with_label(Some(file.span.clone()), "repository file source declared here"),
                        );
                    }
                    if !admissible_destination(match &file.destination {
                        WorkspaceDestination::Symlink { path }
                        | WorkspaceDestination::TrackedCopy { path }
                        | WorkspaceDestination::ManagedBlock { path, .. } => path,
                    }) {
                        diagnostics.push(
                            Diagnostic::error(
                                codes::BAD_DESTINATION,
                                format!(
                                    "profile `{}` file destination {:?} must be absolute or start with ~/ \
                                     and use normalized segments — no NUL, \".\", \"..\", empty or trailing segments",
                                    profile.name,
                                    match &file.destination {
                                        WorkspaceDestination::Symlink { path }
                                        | WorkspaceDestination::TrackedCopy { path }
                                        | WorkspaceDestination::ManagedBlock { path, .. } => path,
                                    }
                                ),
                            )
                            .with_label(Some(file.span.clone()), "destination declared here"),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// A repo-file origin always names a captured file, never an ambient
/// absolute path or a directory selector. This is lexical admission;
/// realization must additionally resolve through a root-pinned snapshot.
fn admissible_repo_file(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// A destination policy path must be absolute or `~/`-prefixed with
/// normalized segments — the module grammar's E102 rule plus the
/// selector segment rules, so no profile file can address `..`, a
/// bare `~`, a relative path or an empty/trailing segment.
fn admissible_destination(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix('/')) else {
        return false;
    };
    !rest.is_empty()
        && !rest.ends_with('/')
        && !rest.contains('\0')
        && rest
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// The decoded wire is not necessarily produced by the TypeScript
/// constructor: a direct --ir caller cannot bypass the literal-body
/// boundary. A source map names the original script line after dedent;
/// without one, label the command rather than inventing a line.
fn check_bash_body(command: &WorkspaceCommand, diagnostics: &mut Vec<Diagnostic>) {
    let WorkspaceCommand::RunBash {
        span,
        body,
        line_map,
        ..
    } = command
    else {
        return;
    };
    let Some(index) = body.find("${") else {
        return;
    };
    let generated_line = body[..index].bytes().filter(|byte| *byte == b'\n').count();
    let mut site = span.clone();
    let label = match line_map
        .get(generated_line)
        .copied()
        .filter(|line| *line > 0)
    {
        Some(source_line) => {
            site.line = source_line;
            site.col = None;
            "interpolation in original Bash body"
        }
        None => "command declared here (no script source map)",
    };
    diagnostics.push(
        Diagnostic::error(
            codes::INVALID_WORKSPACE_VALUE,
            "run_bash body contains `${`, which is not a literal script declaration; \
             pass dynamic values through typed environment/argv bindings",
        )
        .with_label(Some(site), label),
    );
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
    fn invalid_nested_command_coordinates_remain_source_labeled() {
        let task = r#"{
            "kind": "task", "name": "bad", "span": {"file": "grip.ts", "line": 5},
            "run": {"kind": "exec", "span": {"file": "source.ts", "line": 0, "col": 0},
                    "argv": [{"kind": "literal", "value": "true"}]}}"#;
        let diagnostics = crate::check(&doc(&format!("{RECIPE},{PACKAGE},{task}"))).unwrap_err();
        let malformed: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == codes::BAD_WORKSPACE_SPAN)
            .collect();
        assert_eq!(
            malformed.len(),
            2,
            "both invalid coordinates must be reported"
        );
        for diagnostic in malformed {
            assert_eq!(diagnostic.labels.len(), 1);
            let span = diagnostic.labels[0].span.as_ref().unwrap();
            assert_eq!(
                (span.file.as_str(), span.line, span.col),
                ("source.ts", 0, Some(0))
            );
        }
    }

    #[test]
    fn escaping_profile_file_destinations_reject_at_the_declaration() {
        let profile = |path: &str| {
            format!(
                r#"{{
            "kind": "profile", "name": "p", "span": {{"file": "grip.ts", "line": 2}},
            "files": [{{
                "span": {{"file": "grip.ts", "line": 3}},
                "content": {{"kind": "literal", "text": "x\n"}},
                "destination": {{"kind": "tracked_copy", "path": "{path}"}}}}]}}"#
            )
        };
        for admissible in ["~/.vimrc", "/etc/gripsack/tool.conf"] {
            check(&doc(&profile(admissible))).unwrap();
        }
        for escaping in [
            "relative/path",
            "~",
            "~/",
            "~/..",
            "~/a/../b",
            "/../etc/passwd",
            "/a//b",
            "/trailing/",
            "~user/x",
        ] {
            let diagnostics = check(&doc(&profile(escaping))).unwrap_err();
            let rejection = diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == codes::BAD_DESTINATION)
                .unwrap_or_else(|| panic!("{escaping:?} must fail destination admission"));
            assert_eq!(rejection.labels.len(), 1);
            assert_eq!(rejection.labels[0].span.as_ref().unwrap().line, 3);
        }
    }

    #[test]
    fn decoded_repo_file_origin_rejects_paths_outside_the_captured_repository() {
        let profile = |path: &str| {
            let path = serde_json::to_string(path).unwrap();
            format!(
                r#"{{
                    "kind": "profile", "name": "p", "span": {{"file": "grip.ts", "line": 2}},
                    "files": [{{
                        "span": {{"file": "grip.ts", "line": 3}},
                        "source": {{"kind": "repo_file", "path": {path}}},
                        "content": {{"kind": "identity"}},
                        "destination": {{"kind": "tracked_copy", "path": "~/.config/tool"}}}}]}}"#
            )
        };
        for path in [".config/tool", "cfg/vimrc", "pkg/a..b"] {
            check(&doc(&profile(path))).unwrap_or_else(|diags| panic!("{path:?}: {diags:?}"));
        }
        for path in [
            "../outside",
            "/etc/passwd",
            "cfg/./settings",
            "cfg//settings",
            "cfg/",
            ".",
            "cfg\\..\\private",
            "cfg/\0private",
        ] {
            let diagnostics = check(&doc(&profile(path))).unwrap_err();
            let source = diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == codes::INVALID_WORKSPACE_VALUE)
                .unwrap_or_else(|| panic!("{path:?} escaped repository admission"));
            assert!(
                source.message.contains("repository-relative"),
                "{path:?}: {source:?}"
            );
            assert_eq!(source.labels[0].span.as_ref().unwrap().line, 3);
        }
    }

    #[test]
    fn decoded_bash_interpolation_rejects_at_original_script_line() {
        let script = r#"{
            "kind": "task", "name": "script", "span": {"file": "grip.ts", "line": 5},
            "run": {"kind": "run_bash", "span": {"file": "grip.ts", "line": 8},
                    "interpreter": {"kind": "package_command", "package": "hello", "command": "hello"},
                    "body": "echo first\necho ${EVIL}", "line_map": [20, 23]}}"#;
        let invalid = crate::check(&doc(&format!("{RECIPE},{PACKAGE},{script}"))).unwrap_err();
        let body = invalid
            .iter()
            .find(|diagnostic| diagnostic.code == codes::INVALID_WORKSPACE_VALUE)
            .expect("a decoded Bash interpolation must fail before realization");
        assert_eq!(body.labels[0].span.as_ref().unwrap().line, 23);
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
    #[test]
    fn incompatible_abi_and_unsafe_prefix_label_the_declaration() {
        let recipe = RECIPE.replace(
            r#""target": {"os": "linux", "arch": "x86_64"}"#,
            r#""target": {"os": "linux", "arch": "x86_64", "abi": "darwin"}"#,
        );
        let invalid = crate::check(&doc(&recipe)).unwrap_err();
        let abi = invalid
            .iter()
            .find(|d| d.code == codes::INVALID_WORKSPACE_VALUE)
            .expect("incompatible ABI rejected");
        assert_eq!(abi.labels[0].span.as_ref().unwrap().line, 2);

        let package = PACKAGE.replace(
            r#""layout": {"kind": "relocatable"}"#,
            r#""layout": {"kind": "fixed_prefix", "prefix": "/opt/../escape"}"#,
        );
        let invalid = crate::check(&doc(&format!("{RECIPE},{package}"))).unwrap_err();
        let prefix = invalid
            .iter()
            .find(|d| d.code == codes::INVALID_WORKSPACE_VALUE)
            .expect("escaping install prefix rejected");
        assert_eq!(prefix.labels[0].span.as_ref().unwrap().line, 3);
    }
}
