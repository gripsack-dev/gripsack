//! Workspace catalog admission (plan/0052 §2.1–2.2): one typed
//! projection collects named-output edges and command-context
//! violations. Catalog, span and value checks run before resolved
//! references, context checks and dependency-cycle admission.
//! Versioned module maps carry no workspace; a historical v4
//! workspace is projected temporarily for read-only validation.
//! Execution capability is a separate E124 gate.

mod context;
mod cycles;
mod destinations;
mod graph;
mod names;
mod policy;
mod refs;
mod span_value;

use crate::diagnostic::Diagnostic;
use crate::model::Ir;

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    if let Some(workspace) = &ir.workspace {
        check_workspace(workspace, diagnostics);
    }
    if let Some(legacy) = &ir.workspace_v4 {
        let validation_only = legacy.validation_projection();
        check_workspace(&validation_only, diagnostics);
        legacy.check_bindings(diagnostics);
    }
}

fn check_workspace(workspace: &crate::workspace::Workspace, diagnostics: &mut Vec<Diagnostic>) {
    let catalog = names::check(workspace, diagnostics);
    span_value::check(workspace, diagnostics);
    destinations::check(workspace, diagnostics);
    let projection = graph::collect(workspace);
    refs::check(&projection, &catalog, diagnostics);
    context::check(&projection, diagnostics);
    cycles::check(&projection, &catalog, diagnostics);
    if diagnostics.is_empty() {
        policy::check(workspace, &projection, &catalog, diagnostics);
    }
}

/// Shared fixtures for the submodules' unit tests.
#[cfg(test)]
pub(crate) mod testutil {
    pub fn doc(outputs: &str) -> String {
        format!(
            r#"{{"ir_version": 5, "host": {{"os": "linux", "arch": "x86_64"}}, "workspace": {{"span": {{"file": "grip.ts", "line": 1}}, "outputs": [{outputs}]}}}}"#
        )
    }

    pub const RECIPE: &str = r#"{
        "kind": "recipe", "name": "build", "span": {"file": "grip.ts", "line": 2},
        "source": {"fetch": {"kind": "tarball", "url": "https://example.test/src.tgz"}, "span": {"file": "grip.ts", "line": 2}},
        "execution": {"kind": "host", "access": "unconfined"}, "output_kind": "tree",
        "target": {"os": "linux", "arch": "x86_64"}}"#;

    pub const PACKAGE: &str = r#"{
        "kind": "package", "name": "hello", "span": {"file": "grip.ts", "line": 3},
        "producer": {"kind": "recipe", "recipe": "build"}, "commands": {"hello": "bin/hello"},
        "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}}"#;

    pub fn code_of(json: &str) -> Vec<String> {
        crate::check(json)
            .unwrap_err()
            .iter()
            .map(|d| d.code.to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{PACKAGE, RECIPE, doc};
    use crate::{check, codes, parse};

    #[test]
    fn package_and_recipe_workspace_admits() {
        let ir = check(&doc(&format!("{RECIPE},{PACKAGE}"))).unwrap();
        let workspace = ir.workspace.as_ref().unwrap();
        assert_eq!(workspace.outputs.len(), 2);
        assert_eq!(workspace.outputs[0].name(), "build");
        assert_eq!(workspace.outputs[0].kind(), "recipe");
        assert_eq!(workspace.outputs[1].name(), "hello");
        assert_eq!(workspace.outputs[1].kind(), "package");
        assert!(ir.modules.is_empty());
    }

    #[test]
    fn provider_backed_package_needs_no_recipe_output() {
        // plan/0052 §2.2: producer = recipe ref OR provider — a lone
        // provider-backed package is one output, no synthetic recipe.
        let package = r#"{
            "kind": "package", "name": "ripgrep", "span": {"file": "grip.ts", "line": 2},
            "producer": {"kind": "provider", "provider": {
                "fetch": {"kind": "github_release", "repo": "BurntSushi/ripgrep", "asset": "ripgrep.tar.gz"},
                "span": {"file": "grip.ts", "line": 2}}},
            "commands": {"rg": "bin/rg"},
            "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}}"#;
        let ir = check(&doc(package)).unwrap();
        assert_eq!(ir.workspace.as_ref().unwrap().outputs.len(), 1);
    }

    #[test]
    fn provider_fetch_fields_never_drop_silently() {
        // unknown field inside the provider's nested fetch spec — the
        // tagged pre-pass visits producer/provider/fetch
        let package = r#"{
            "kind": "package", "name": "ripgrep", "span": {"file": "grip.ts", "line": 2},
            "producer": {"kind": "provider", "provider": {
                "fetch": {"kind": "github_release", "repo": "BurntSushi/ripgrep", "asset": "x", "baseUrl": "https://evil.test"},
                "span": {"file": "grip.ts", "line": 2}}},
            "commands": {"rg": "bin/rg"},
            "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}}"#;
        assert_eq!(parse(&doc(package)).unwrap_err().code, codes::MALFORMED);
        // unknown producer kind
        let alien = PACKAGE.replace(
            r#"{"kind": "recipe", "recipe": "build"}"#,
            r#"{"kind": "magic", "recipe": "build"}"#,
        );
        assert_eq!(
            parse(&doc(&format!("{RECIPE},{alien}"))).unwrap_err().code,
            codes::MALFORMED
        );
        // unknown field on the producer union itself
        let extra = PACKAGE.replace(
            r#""recipe": "build"}"#,
            r#""recipe": "build", "branch": "main"}"#,
        );
        assert_eq!(
            parse(&doc(&format!("{RECIPE},{extra}"))).unwrap_err().code,
            codes::MALFORMED
        );
    }

    #[test]
    fn profile_with_literal_file_needs_no_synthetic_package() {
        let profile = r#"{
            "kind": "profile", "name": "dotfiles", "span": {"file": "grip.ts", "line": 2},
            "files": [{
                "span": {"file": "grip.ts", "line": 3},
                "content": {"kind": "literal", "text": "set number\n"},
                "destination": {"kind": "tracked_copy", "path": "~/.vimrc"}}]}"#;
        check(&doc(profile)).unwrap();
    }

    #[test]
    fn envelope_shape_is_version_dispatched() {
        // v3 keeps its strict root: workspace is an unknown extra there
        let v3_workspace = r#"{"ir_version": 3, "modules": {}, "workspace": {"span": {"file": "g", "line": 1}, "outputs": []}}"#;
        assert_eq!(parse(v3_workspace).unwrap_err().code, codes::MALFORMED);
        // v3 still requires modules
        let v3_bare = r#"{"ir_version": 3}"#;
        assert_eq!(parse(v3_bare).unwrap_err().code, codes::MALFORMED);
        // v4: both workspace and modules
        let both = r#"{"ir_version": 4, "host": {"os": "linux", "arch": "x86_64"}, "modules": {}, "workspace": {"span": {"file": "g", "line": 1}, "outputs": []}}"#;
        assert_eq!(parse(both).unwrap_err().code, codes::MALFORMED);
        // v4: neither
        let neither = r#"{"ir_version": 4, "host": {"os": "linux", "arch": "x86_64"}}"#;
        assert_eq!(parse(neither).unwrap_err().code, codes::MALFORMED);
        // v4: host facts are required
        let no_host =
            r#"{"ir_version": 4, "workspace": {"span": {"file": "g", "line": 1}, "outputs": []}}"#;
        assert_eq!(parse(no_host).unwrap_err().code, codes::MALFORMED);
        // v4 legacy modules branch still parses
        let legacy =
            r#"{"ir_version": 4, "host": {"os": "linux", "arch": "x86_64"}, "modules": {}}"#;
        let ir = parse(legacy).unwrap();
        assert!(ir.workspace.is_none());
        let v5_both = both.replace(r#""ir_version": 4"#, r#""ir_version": 5"#);
        assert_eq!(parse(&v5_both).unwrap_err().code, codes::MALFORMED);
        let v5_modules = legacy.replace(r#""ir_version": 4"#, r#""ir_version": 5"#);
        assert!(parse(&v5_modules).is_ok());
        // Out-of-range versions remain E100, not a silent fallback.
        let future = r#"{"ir_version": 6, "modules": {}}"#;
        assert_eq!(parse(future).unwrap_err().code, codes::VERSION);
    }

    #[test]
    fn unknown_workspace_fields_label_the_declaring_span() {
        // The e2e mirror (A1-06): an injected field on a profile output
        // is rejected with the profile's own declaration span.
        let profile = r#"{
            "kind": "profile", "name": "p", "span": {"file": "gripsack.ts", "line": 9},
            "unexpected_effect": true}"#;
        let diagnostic = parse(&doc(profile)).unwrap_err();
        assert_eq!(diagnostic.code, codes::MALFORMED);
        let span = diagnostic.labels[0]
            .span
            .as_ref()
            .expect("output span labeled");
        assert_eq!((span.file.as_str(), span.line), ("gripsack.ts", 9));

        // A command-level extra labels the command's own span, and an
        // argument-level extra falls back to that same command span.
        let task = r#"{
            "kind": "task", "name": "t", "span": {"file": "gripsack.ts", "line": 4},
            "run": {"kind": "exec", "span": {"file": "gripsack.ts", "line": 5},
                    "argv": [{"kind": "literal", "value": "x", "shell": true}]}}"#;
        let diagnostic = parse(&doc(task)).unwrap_err();
        let span = diagnostic.labels[0]
            .span
            .as_ref()
            .expect("command span labeled");
        assert_eq!((span.file.as_str(), span.line), ("gripsack.ts", 5));

        // A fetch-spec extra labels the workspaceFetch source span —
        // not the recipe's (recipe declares at line 2, source at line 3).
        let recipe = r#"{
            "kind": "recipe", "name": "build", "span": {"file": "gripsack.ts", "line": 2},
            "source": {"fetch": {"kind": "tarball", "url": "https://example.test/s.tgz", "baseUrl": "https://evil.test"},
                       "span": {"file": "gripsack.ts", "line": 3}},
            "execution": {"kind": "host", "access": "unconfined"}, "output_kind": "tree",
            "target": {"os": "linux", "arch": "x86_64"}}"#;
        let diagnostic = parse(&doc(recipe)).unwrap_err();
        let span = diagnostic.labels[0]
            .span
            .as_ref()
            .expect("source span labeled");
        assert_eq!((span.file.as_str(), span.line), ("gripsack.ts", 3));
    }

    #[test]
    fn unknown_workspace_fields_never_drop_silently() {
        // unknown field on an output (tagged union — pass 1.5 must catch it)
        let extra = PACKAGE.replace(
            r#""layout": {"kind": "relocatable"}"#,
            r#""layout": {"kind": "relocatable"}, "stage": "deploy""#,
        );
        assert_eq!(
            parse(&doc(&format!("{RECIPE},{extra}"))).unwrap_err().code,
            codes::MALFORMED
        );
        // unknown output kind
        let alien = r#"{"kind": "widget", "name": "w", "span": {"file": "grip.ts", "line": 2}}"#;
        assert_eq!(parse(&doc(alien)).unwrap_err().code, codes::MALFORMED);
        // unknown field inside the recipe's nested fetch spec
        let bad_fetch = RECIPE.replace(
            r#""url": "https://example.test/src.tgz""#,
            r#""url": "https://example.test/src.tgz", "baseUrl": "https://evil.test""#,
        );
        assert_eq!(
            parse(&doc(&format!("{bad_fetch},{PACKAGE}")))
                .unwrap_err()
                .code,
            codes::MALFORMED
        );
        // unknown field on a plain struct (platform) — serde deny_unknown
        let bad_platform = RECIPE.replace(
            r#""arch": "x86_64"}"#,
            r#""arch": "x86_64", "kernel": "6.1"}"#,
        );
        assert_eq!(
            parse(&doc(&format!("{bad_platform},{PACKAGE}")))
                .unwrap_err()
                .code,
            codes::MALFORMED
        );
        // unknown field on an exec argument
        let bad_arg = r#"{
            "kind": "task", "name": "t", "span": {"file": "grip.ts", "line": 4},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                    "argv": [{"kind": "literal", "value": "x", "shell": true}]}}"#;
        assert_eq!(parse(&doc(bad_arg)).unwrap_err().code, codes::MALFORMED);
        // future workspace fields stay rejected until their milestone
        let future = r#"{"ir_version": 4, "host": {"os": "linux", "arch": "x86_64"}, "workspace": {"span": {"file": "g", "line": 1}, "name": "w", "outputs": []}}"#;
        assert_eq!(parse(future).unwrap_err().code, codes::MALFORMED);
    }
}
