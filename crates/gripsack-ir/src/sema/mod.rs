//! Structural sema (0004 §4): ordered, composable passes.
//!
//! Each pass is one small function in its own module with the same
//! signature — `check(&Ir, &mut Vec<Diagnostic>)` — and one concern.
//! To add a check: write the pass, add one line to `PASSES`, add a
//! test. Passes never short-circuit internally: collect everything,
//! so one bad module never hides another.

mod deps;
mod destinations;
mod features;
mod resources;
mod steps;
mod verify_paths;

use crate::diagnostic::{Diagnostic, Severity};
use crate::model::Ir;
use crate::parse::parse;

/// The passes, in execution order.
const PASSES: &[fn(&Ir, &mut Vec<Diagnostic>)] = &[
    steps::check,
    deps::check,
    destinations::check,
    resources::check,
    features::check,
    verify_paths::check,
];

/// Pass 2 — run every sema pass, collecting all diagnostics.
pub fn run(ir: &Ir) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for pass in PASSES {
        pass(ir, &mut diagnostics);
    }
    diagnostics
}

/// Parse + sema in one call (the CLI's usual path). Collects
/// everything pass 2 finds — one bad module never hides another.
pub fn check(json: &str) -> Result<Ir, Vec<Diagnostic>> {
    let ir = parse(json).map_err(|d| vec![d])?;
    let diagnostics = run(&ir);
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(diagnostics);
    }
    Ok(ir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::codes;
    use crate::model::*;

    const EXAMPLE: &str = r#"{
        "ir_version": 1,
        "host": {"os": "linux", "arch": "x86_64", "tags": ["gui"]},
        "modules": {
            "helix": {
                "fetch": {"kind": "github_release", "repo": "helix-editor/helix",
                           "asset": "helix-{version}-x86_64-linux.tar.xz"},
                "install": [{"from": "bin/hx", "to": "~/.local/bin/hx", "mode": "owned"}],
                "config": [{"from": "config.toml", "to": "~/.config/helix/config.toml",
                            "mode": "tracked_copy"}],
                "depends": [{"module": "git", "edge": "runtime"}],
                "activate": [{"trigger": "post_activate",
                              "kind": "service", "name": "syncthing", "user": true}],
                "span": {"file": "modules/helix.py", "line": 4, "col": 1}
            },
            "git": {
                "fetch": {"kind": "tarball", "url": "https://example.invalid/git.tar.xz"}
            }
        }
    }"#;

    #[test]
    fn parses_and_validates_example() {
        let ir = check(EXAMPLE).unwrap();
        assert_eq!(ir.ir_version, 1);
        assert_eq!(ir.modules.len(), 2);
        let helix = &ir.modules["helix"];
        assert!(matches!(helix.fetch, Some(FetchSpec::GithubRelease { .. })));
        assert_eq!(helix.config[0].mode, Ownership::TrackedCopy);
        assert_eq!(helix.span.as_ref().unwrap().line, 4);
        let again = serde_json::to_string(&ir).unwrap();
        check(&again).unwrap();
    }

    #[test]
    fn unknown_dependency_carries_span_and_code() {
        let bad = EXAMPLE.replace(r#""module": "git""#, r#""module": "nope""#);
        let diagnostics = check(&bad).unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        let d = &diagnostics[0];
        assert_eq!(d.code, codes::UNKNOWN_DEPENDENCY);
        let rendered = d.to_string();
        assert!(rendered.contains("error[E101]"));
        assert!(rendered.contains("modules/helix.py:4:1"));
    }

    #[test]
    fn collects_multiple_diagnostics() {
        let bad = EXAMPLE
            .replace(r#""module": "git""#, r#""module": "nope""#)
            .replace("~/.local/bin/hx", "bin/hx-elsewhere");
        let diagnostics = check(&bad).unwrap_err();
        assert_eq!(diagnostics.len(), 2);
        let codes: Vec<_> = diagnostics.iter().map(|d| d.code.as_ref()).collect();
        assert!(codes.contains(&codes::UNKNOWN_DEPENDENCY));
        assert!(codes.contains(&codes::BAD_DESTINATION));
    }

    #[test]
    fn rejects_wrong_version() {
        let bad = EXAMPLE.replace(r#""ir_version": 1"#, r#""ir_version": 99"#);
        let diagnostics = check(&bad).unwrap_err();
        assert_eq!(diagnostics[0].code, codes::VERSION);
    }

    #[test]
    fn malformed_json_is_e000() {
        let diagnostics = check("{not json").unwrap_err();
        assert_eq!(diagnostics[0].code, codes::MALFORMED);
        assert!(diagnostics[0].help.is_some());
    }

    #[test]
    fn ownership_and_edge_defaults() {
        let e: Entry = serde_json::from_str(r#"{"from":"a","to":"/b"}"#).unwrap();
        assert!(matches!(e.mode, Ownership::Owned));
        let d: Dependency = serde_json::from_str(r#"{"module":"m"}"#).unwrap();
        assert!(matches!(d.edge, EdgeKind::Runtime));
    }

    #[test]
    fn dotfiles_only_module_needs_no_source() {
        // 0006 §2 level 1: a module that only manages configs.
        let json = r#"{
            "ir_version": 1,
            "modules": {
                "helix": {
                    "config": [{"from": "config.toml",
                                "to": "~/.config/helix/config.toml",
                                "mode": "tracked_copy"}]
                }
            }
        }"#;
        let ir = check(json).unwrap();
        assert_eq!(ir.modules["helix"].fetch, None);
        assert_eq!(ir.modules["helix"].config.len(), 1);
    }

    const STEPPED: &str = r#"{
        "ir_version": 1,
        "modules": {
            "helix": {
                "steps": [
                    {"id": "fetch", "action": {"kind": "fetch",
                     "fetch": {"kind": "tarball", "url": "https://example.invalid/h.tar.xz"}},
                     "phase": "fetch"},
                    {"id": "patch", "action": {"kind": "custom_shell", "script": "true"},
                     "needs": ["fetch"], "phase": "custom"}
                ]
            },
            "rust": {
                "fetch": {"kind": "tarball", "url": "https://example.invalid/r.tar.xz"}
            }
        }
    }"#;

    #[test]
    fn explicit_steps_validate() {
        check(STEPPED).unwrap();
    }

    #[test]
    fn run_action_parses_with_outputs() {
        let json = STEPPED.replace(
            r#"{"id": "patch", "action": {"kind": "custom_shell", "script": "true"},
                     "needs": ["fetch"], "phase": "custom"}"#,
            r#"{"id": "make", "action": {"kind": "run",
                     "argv": ["make", "install"], "outputs": ["bin/hx"]},
                     "needs": ["fetch"]}"#,
        );
        let ir = check(&json).unwrap();
        let step = &ir.modules["helix"].steps.as_ref().unwrap()[1];
        match &step.action {
            crate::StepAction::Run { argv, outputs, .. } => {
                assert_eq!(argv, &["make", "install"]);
                assert_eq!(outputs, &["bin/hx"]);
            }
            other => panic!("expected run action, got {other:?}"),
        }
    }

    #[test]
    fn verify_and_retries_roundtrip() {
        let json = STEPPED.replace(
            r#""needs": ["fetch"], "phase": "custom"}"#,
            r#""needs": ["fetch"], "phase": "custom",
             "verify": {"kind": "binary_runs", "path": "bin/hx", "args": ["--version"]},
             "retries": 2}"#,
        );
        let ir = check(&json).unwrap();
        let step = &ir.modules["helix"].steps.as_ref().unwrap()[1];
        assert!(matches!(step.verify, Some(Verify::BinaryRuns { .. })));
        assert_eq!(step.retries, Some(2));
        let again = serde_json::to_string(&ir).unwrap();
        check(&again).unwrap();
    }

    #[test]
    fn cross_module_ref_into_declarative_module() {
        let json = STEPPED.replace(
            r#""needs": ["fetch"]"#,
            r#""needs": ["fetch", "rust:install"]"#,
        );
        check(&json).unwrap();
        let bad = STEPPED.replace(r#""needs": ["fetch"]"#, r#""needs": ["fetch", "rust:wat"]"#);
        let diagnostics = check(&bad).unwrap_err();
        assert!(diagnostics.iter().any(|d| d.code == codes::UNKNOWN_STEP));
    }

    #[test]
    fn steps_plus_declarative_fields_is_e103() {
        let bad = STEPPED.replace(
            r#""steps": ["#,
            r#""fetch": {"kind": "file", "path": "/x"}, "steps": ["#,
        );
        let diagnostics = check(&bad).unwrap_err();
        assert_eq!(diagnostics[0].code, codes::STEPS_WITH_FIELDS);
        assert!(diagnostics[0].help.is_some());
    }

    #[test]
    fn duplicate_and_reserved_step_ids_are_e106() {
        let dup = STEPPED.replace(
            r#"{"id": "patch", "action": {"kind": "custom_shell", "script": "true"},
                     "needs": ["fetch"], "phase": "custom"}"#,
            r#"{"id": "fetch", "action": {"kind": "custom_shell", "script": "true"}}"#,
        );
        assert!(
            check(&dup)
                .unwrap_err()
                .iter()
                .any(|d| d.code == codes::DUPLICATE_STEP)
        );

        let reserved = STEPPED.replace(r#""id": "patch""#, r#""id": "done""#);
        assert!(
            check(&reserved)
                .unwrap_err()
                .iter()
                .any(|d| d.code == codes::DUPLICATE_STEP)
        );
    }

    #[test]
    fn undeclared_resource_is_e107() {
        let json = STEPPED.replace(
            r#""needs": ["fetch"]"#,
            r#""needs": ["fetch"], "resources": ["my-lock"]"#,
        );
        let diagnostics = check(&json).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == codes::UNKNOWN_RESOURCE && d.severity == Severity::Error)
        );
    }

    #[test]
    fn declared_and_builtin_resources_pass() {
        let json = STEPPED
            .replace(
                r#""needs": ["fetch"]"#,
                r#""needs": ["fetch"], "resources": ["my-lock", "cargo-lock"]"#,
            )
            .replace(
                r#""modules": {"#,
                r#""resources": [{"name": "my-lock"}], "modules": {"#,
            );
        check(&json).unwrap();
    }

    #[test]
    fn done_barrier_ref_is_always_valid() {
        let json = STEPPED.replace(
            r#""needs": ["fetch"]"#,
            r#""needs": ["fetch", "rust:done"]"#,
        );
        check(&json).unwrap();
        // and into an explicit-steps module too
        let json = STEPPED.replace(
            r#""needs": ["fetch"]"#,
            r#""needs": ["fetch", "helix:done"]"#,
        );
        check(&json).unwrap();
    }

    #[test]
    fn merge_mode_is_e108_not_a_mid_apply_error() {
        let json = r#"{
            "ir_version": 1,
            "modules": {
                "shell": {
                    "config": [{"from": "rc", "to": "~/.bashrc", "mode": "merge"}]
                }
            }
        }"#;
        let diagnostics = check(json).unwrap_err();
        assert_eq!(diagnostics[0].code, codes::UNSUPPORTED_MODE);
        assert!(diagnostics[0].help.is_some());
    }

    #[test]
    fn destination_shaped_verify_path_is_e109() {
        let json = r#"{
            "ir_version": 1,
            "modules": {
                "gitui": {
                    "config": [{"from": "theme.ron", "to": "~/.config/gitui/theme.ron"}],
                    "verify": {"kind": "file_exists", "path": "~/.config/gitui/theme.ron"}
                }
            }
        }"#;
        let diagnostics = check(json).unwrap_err();
        assert_eq!(diagnostics[0].code, codes::VERIFY_PATH_SHAPE);
        assert!(
            diagnostics[0]
                .help
                .as_ref()
                .unwrap()
                .contains("verify_deployed")
        );
    }

    #[test]
    fn spanless_nodes_still_render() {
        let d = Diagnostic::error(codes::UNKNOWN_DEPENDENCY, "no span here")
            .with_label(None, "module xyz");
        let rendered = d.to_string();
        assert!(rendered.contains("= module xyz"));
    }
}
