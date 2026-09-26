//! Reference admission over the graph projection (E126, E130): every
//! collected edge resolves against the catalog — unknown names and
//! wrong output kinds label both the reference site and the mismatched
//! declaration, `package_command` slots must name an exported command,
//! artifact selectors must be normalized relative paths (E130), and
//! platform-bound edges (package producer, environment/image package
//! selections) require exact target equality and reject `fixed_prefix`
//! selections the wire cannot place (0052 §2.2).

mod bindings;
use bindings::check_binding;

use super::graph::{Edge, Projection, TargetBinding};
use super::names::Catalog;
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::WorkspaceOutput;

pub(super) fn check(projection: &Projection, catalog: &Catalog, diagnostics: &mut Vec<Diagnostic>) {
    for edge in &projection.edges {
        if let Some(selector) = edge.selector {
            check_selector(edge, selector, diagnostics);
        }
        let Some(target) = catalog.outputs.get(edge.to) else {
            diagnostics.push(
                Diagnostic::error(
                    codes::UNKNOWN_WORKSPACE_REF,
                    format!(
                        "{} references unknown workspace output `{}`",
                        edge.relation, edge.to
                    ),
                )
                .with_label(Some(edge.at.clone()), "reference declared here"),
            );
            continue;
        };
        if !edge.expected.contains(&target.kind()) {
            diagnostics.push(
                Diagnostic::error(
                    codes::UNKNOWN_WORKSPACE_REF,
                    format!(
                        "{} must reference a {} output; `{}` is a {} output",
                        edge.relation,
                        edge.expected.join(" or "),
                        edge.to,
                        target.kind()
                    ),
                )
                .with_label(Some(edge.at.clone()), "reference declared here")
                .with_label(
                    Some(target.span().clone()),
                    format!("`{}` declared here", edge.to),
                ),
            );
            continue;
        }
        if let Some(command) = edge.command {
            check_command_export(edge, command, target, diagnostics);
        }
        check_binding(edge, target, diagnostics);
    }
}

/// E130 — an artifact selector is `.` (the whole artifact) or a
/// normalized relative POSIX path: no leading slash, no empty, `.` or
/// `..` segments. Anything else could escape the addressed artifact
/// (mirrors the TS emitter's `asSelector`).
fn check_selector(edge: &Edge, selector: &str, diagnostics: &mut Vec<Diagnostic>) {
    let normalized = !selector.contains('\0')
        && (selector == "."
            || (!selector.starts_with('/')
                && selector
                    .split('/')
                    .all(|segment| !segment.is_empty() && segment != "." && segment != "..")));
    if !normalized {
        diagnostics.push(
            Diagnostic::error(
                codes::INVALID_WORKSPACE_VALUE,
                format!(
                    "{} has invalid artifact selector `{selector}`: a selector is `.` (the \
                     whole artifact) or a normalized relative POSIX path — no NUL, leading `/`, \
                     empty, `.` or `..` segments",
                    edge.relation
                ),
            )
            .with_label(Some(edge.at.clone()), "selector declared here"),
        );
    }
}

/// A `package_command` slot must name a command the package exports.
fn check_command_export(
    edge: &Edge,
    command: &str,
    target: &WorkspaceOutput,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Kind was verified by the caller: a package_command edge expects
    // exactly a package.
    let WorkspaceOutput::Package(package) = target else {
        return;
    };
    if !package.commands.contains_key(command) {
        diagnostics.push(
            Diagnostic::error(
                codes::UNKNOWN_WORKSPACE_REF,
                format!(
                    "package_command references command `{command}`, which package `{}` does not export",
                    edge.to
                ),
            )
            .with_label(Some(edge.at.clone()), "reference declared here")
            .with_label(
                Some(package.span.clone()),
                format!("`{}` declared here", edge.to),
            ),
        );
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
                    "argv": [{"kind": "literal", "value": "x"}]},
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
        let schedule_ok = schedule.replace(r#""task": "hello""#, r#""task": "t""#);
        let task_ok = task.replace(r#""deps": ["missing"]"#, r#""deps": []"#);
        assert!(
            code_of(&doc(&format!(
                "{RECIPE},{PACKAGE},{task_ok},{schedule_ok},{profile}"
            )))
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
            r#""execution": {"kind": "host", "access": "unconfined"}"#,
            r#""checks": ["t"], "execution": {"kind": "host", "access": "unconfined"}"#,
        );
        assert!(
            code_of(&doc(&format!("{recipe},{PACKAGE},{task_ok}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
    }

    #[test]
    fn check_subject_must_resolve_to_an_output() {
        let check = r#"{
            "kind": "check", "name": "verify", "span": {"file": "grip.ts", "line": 4},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                    "argv": [{"kind": "literal", "value": "true"}]},
            "subject": "no-such-output"}"#;
        let diagnostics = crate::check(&doc(check)).unwrap_err();
        let subject = diagnostics
            .iter()
            .find(|d| d.code == codes::UNKNOWN_WORKSPACE_REF)
            .expect("E126 fired");
        assert!(subject.message.contains("no-such-output"));
        assert_eq!(subject.labels[0].span.as_ref().unwrap().line, 4);
        // a subject naming an admitted output — of any kind — resolves
        let ok = check.replace(r#""subject": "no-such-output""#, r#""subject": "verify""#);
        assert!(crate::check(&doc(&ok)).is_ok());
    }

    #[test]
    fn artifact_references_reject_non_artifact_kinds() {
        // tasks carry no artifacts — an artifact_file source naming one
        // labels both the file and the task declaration
        let task = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 9},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 9},
                    "argv": [{"kind": "literal", "value": "true"}]}}"#;
        let profile = r#"{
            "kind": "profile", "name": "p", "span": {"file": "grip.ts", "line": 4},
            "files": [{"span": {"file": "grip.ts", "line": 5},
                       "source": {"kind": "artifact_file", "output": "t", "selector": "x"},
                       "content": {"kind": "identity"},
                       "destination": {"kind": "symlink", "path": "~/.x"}}]}"#;
        let diagnostics = crate::check(&doc(&format!("{task},{profile}"))).unwrap_err();
        let reference = diagnostics
            .iter()
            .find(|d| d.code == codes::UNKNOWN_WORKSPACE_REF)
            .expect("E126 fired");
        let lines: Vec<u32> = reference
            .labels
            .iter()
            .filter_map(|l| l.span.as_ref().map(|s| s.line))
            .collect();
        assert_eq!(lines, vec![5, 9], "reference site and target labeled");
        // recipes and packages DO carry artifacts
        let ok = profile.replace(r#""output": "t""#, r#""output": "hello""#);
        assert!(crate::check(&doc(&format!("{RECIPE},{PACKAGE},{ok}"))).is_ok());
    }

    #[test]
    fn artifact_selectors_must_be_normalized_relative_paths() {
        let profile = |selector: &str| {
            format!(
                r#"{{
                "kind": "profile", "name": "p", "span": {{"file": "grip.ts", "line": 4}},
                "files": [{{"span": {{"file": "grip.ts", "line": 5}},
                           "source": {{"kind": "artifact_file", "output": "hello", "selector": "{selector}"}},
                           "content": {{"kind": "identity"}},
                           "destination": {{"kind": "symlink", "path": "~/.x"}}}}]}}"#
            )
        };
        for bad in [
            "../secrets",
            "/abs",
            "a//b",
            "a/./b",
            "a/",
            "./x",
            r"\u0000",
        ] {
            let diagnostics =
                crate::check(&doc(&format!("{RECIPE},{PACKAGE},{}", profile(bad)))).unwrap_err();
            let diagnostic = diagnostics
                .iter()
                .find(|d| d.code == codes::INVALID_WORKSPACE_VALUE)
                .unwrap_or_else(|| panic!("selector {bad:?} rejected with E130"));
            assert!(diagnostic.message.contains("selector"));
            assert_eq!(
                diagnostic.labels[0].span.as_ref().unwrap().line,
                5,
                "selector {bad:?} labels the file span"
            );
        }
        for good in [".", "bin/hello", "a/b/c"] {
            crate::check(&doc(&format!("{RECIPE},{PACKAGE},{}", profile(good))))
                .unwrap_or_else(|ds| panic!("selector {good:?} admitted: {ds:?}"));
        }
    }

    #[test]
    fn producer_target_mismatch_labels_both_sites() {
        let package = PACKAGE.replace(
            r#""target": {"os": "linux", "arch": "x86_64"}"#,
            r#""target": {"os": "macos", "arch": "aarch64"}"#,
        );
        let diagnostics = crate::check(&doc(&format!("{RECIPE},{package}"))).unwrap_err();
        let mismatch = diagnostics
            .iter()
            .find(|d| d.code == codes::UNKNOWN_WORKSPACE_REF && d.message.contains("target"))
            .expect("E126 target mismatch fired");
        let lines: Vec<u32> = mismatch
            .labels
            .iter()
            .filter_map(|l| l.span.as_ref().map(|s| s.line))
            .collect();
        assert_eq!(lines, vec![3, 2], "package and recipe spans labeled");
        // ABI drift alone is a mismatch too (exact equality)
        let abi_recipe = RECIPE.replace(
            r#""target": {"os": "linux", "arch": "x86_64"}"#,
            r#""target": {"os": "linux", "arch": "x86_64", "abi": "musl"}"#,
        );
        assert!(crate::check(&doc(&format!("{abi_recipe},{PACKAGE}"))).is_err());
    }

    #[test]
    fn fixed_prefix_packages_cannot_be_selected_without_prefix() {
        let package = PACKAGE.replace(
            r#""layout": {"kind": "relocatable"}"#,
            r#""layout": {"kind": "fixed_prefix", "prefix": "/opt/tool"}"#,
        );
        let env = r#"{
            "kind": "environment", "name": "dev", "span": {"file": "grip.ts", "line": 6},
            "packages": ["hello"], "target": {"os": "linux", "arch": "x86_64"}}"#;
        let diagnostics = crate::check(&doc(&format!("{RECIPE},{package},{env}"))).unwrap_err();
        let layout = diagnostics
            .iter()
            .find(|d| d.code == codes::UNKNOWN_WORKSPACE_REF && d.message.contains("fixed_prefix"))
            .expect("E126 fixed_prefix selection fired");
        let lines: Vec<u32> = layout
            .labels
            .iter()
            .filter_map(|l| l.span.as_ref().map(|s| s.line))
            .collect();
        assert_eq!(lines, vec![6, 3], "environment and package spans labeled");
        // unselected fixed_prefix packages still admit — the gate is on
        // consumer selections, not the declaration
        assert!(crate::check(&doc(&format!("{RECIPE},{package}"))).is_ok());
        // runtime closure is not a selection either
        let base = PACKAGE.replace(r#""name": "hello""#, r#""name": "base""#);
        let with_runtime = package.replace(
            r#""commands": {"hello": "bin/hello"}"#,
            r#""commands": {"hello": "bin/hello"}, "runtime": ["base"]"#,
        );
        assert!(crate::check(&doc(&format!("{RECIPE},{base},{with_runtime}"))).is_ok());
        let bound = env.replace(
            r#""packages": ["hello"]"#,
            r#""packages": ["hello"], "prefix": "/opt/tool""#,
        );
        crate::check(&doc(&format!("{RECIPE},{package},{bound}")))
            .expect("a declared matching destination admits");
        let wrong = env.replace(
            r#""packages": ["hello"]"#,
            r#""packages": ["hello"], "prefix": "/different""#,
        );
        assert!(
            code_of(&doc(&format!("{RECIPE},{package},{wrong}")))
                .contains(&codes::UNKNOWN_WORKSPACE_REF.into())
        );
    }

    #[test]
    fn selection_target_mismatch_labels_both_sites() {
        let env = r#"{
            "kind": "environment", "name": "dev", "span": {"file": "grip.ts", "line": 6},
            "packages": ["hello"], "target": {"os": "macos", "arch": "aarch64"}}"#;
        let diagnostics = crate::check(&doc(&format!("{RECIPE},{PACKAGE},{env}"))).unwrap_err();
        let mismatch = diagnostics
            .iter()
            .find(|d| d.code == codes::UNKNOWN_WORKSPACE_REF && d.message.contains("target"))
            .expect("E126 selection target mismatch fired");
        let lines: Vec<u32> = mismatch
            .labels
            .iter()
            .filter_map(|l| l.span.as_ref().map(|s| s.line))
            .collect();
        assert_eq!(lines, vec![6, 3], "environment and package spans labeled");
    }
}
