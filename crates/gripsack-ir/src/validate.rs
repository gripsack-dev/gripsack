use crate::diagnostic::{codes, Diagnostic, Label, Severity};
use crate::model::{Build, Ir};
use crate::step::{BARRIER_STEP_ID, KNOWN_RESOURCES, SYNTHESIZED_STEP_IDS};

/// The only IR version this core accepts (for now).
pub const IR_VERSION: u32 = 1;

/// Parse + validate in one call (the CLI's usual path). Collects
/// everything pass 2 finds — one bad module never hides another.
pub fn check(json: &str) -> Result<Ir, Vec<Diagnostic>> {
    let ir = parse(json).map_err(|d| vec![d])?;
    let diagnostics = validate(&ir);
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(diagnostics);
    }
    Ok(ir)
}

/// Pass 1 — parse (0004 §4): syntax + version gate.
pub fn parse(json: &str) -> Result<Ir, Diagnostic> {
    let ir: Ir = serde_json::from_str(json).map_err(|e| {
        Diagnostic::error(codes::MALFORMED, format!("invalid IR JSON: {e}"))
            .with_help("the frontend emitted malformed IR — this is a frontend bug")
    })?;
    if ir.ir_version != IR_VERSION {
        return Err(Diagnostic::error(
            codes::VERSION,
            format!(
                "unsupported ir_version {} (this core accepts {IR_VERSION})",
                ir.ir_version
            ),
        ));
    }
    Ok(ir)
}

/// Pass 2 — structural sema (0004 §4): collect all diagnostics.
pub fn validate(ir: &Ir) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    validate_steps(ir, &mut diagnostics);
    for (name, module) in &ir.modules {
        for dep in &module.depends {
            if !ir.modules.contains_key(&dep.module) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_DEPENDENCY,
                        format!("module {name:?} depends on unknown module {:?}", dep.module),
                    )
                    .with_label(
                        dep.span.clone().or_else(|| module.span.clone()),
                        "dependency declared here",
                    ),
                );
            }
        }
        for entry in module.install.iter().chain(module.config.iter()) {
            if !(entry.to.starts_with('/') || entry.to.starts_with("~/")) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::BAD_DESTINATION,
                        format!(
                            "module {name:?}: destination {:?} must be absolute or start with ~/",
                            entry.to
                        ),
                    )
                    .with_label(
                        entry.span.clone().or_else(|| module.span.clone()),
                        "entry declared here",
                    ),
                );
            }
        }
    }
    diagnostics
}

/// Steps pass (0007 §6): shape, refs, ids.
fn validate_steps(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        let Some(steps) = &module.steps else {
            continue;
        };
        // E103: explicit steps + any declarative field.
        let mixed = module.fetch.is_some()
            || module.build != Build::None
            || !module.install.is_empty()
            || !module.config.is_empty()
            || !module.activate.is_empty();
        if mixed {
            diagnostics.push(
                Diagnostic::error(
                    codes::STEPS_WITH_FIELDS,
                    format!(
                        "module {name:?} mixes `steps` with declarative fields \
                         (fetch/build/install/config/activate)"
                    ),
                )
                .with_label(module.span.clone(), "module declared here")
                .with_help("pick one shape: fields (expanded for you) or explicit steps"),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for step in steps {
            // E106: duplicate or reserved ids.
            if !seen.insert(step.id.as_str()) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_STEP,
                        format!("module {name:?}: duplicate step id {:?}", step.id),
                    )
                    .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                );
            }
            if step.id == BARRIER_STEP_ID {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_STEP,
                        format!(
                            "module {name:?}: step id {BARRIER_STEP_ID:?} is reserved \
                             (the module's barrier step)"
                        ),
                    )
                    .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                );
            }
            // E104: unknown step refs.
            for need in &step.needs {
                let unknown = match need.split_once(':') {
                    Some((target_module, target_step)) => match ir.modules.get(target_module) {
                        None => true,
                        Some(target) => match &target.steps {
                            Some(target_steps) => !target_steps.iter().any(|s| s.id == target_step),
                            None => !SYNTHESIZED_STEP_IDS.contains(&target_step),
                        },
                    },
                    None => !steps.iter().any(|s| s.id == *need),
                };
                if unknown {
                    diagnostics.push(
                        Diagnostic::error(
                            codes::UNKNOWN_STEP,
                            format!(
                                "module {name:?}: step {:?} needs unknown step {need:?}",
                                step.id
                            ),
                        )
                        .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                    );
                }
            }
            // W201: unknown resource names.
            for resource in &step.resources {
                if !KNOWN_RESOURCES.contains(&resource.as_str()) {
                    diagnostics.push(Diagnostic {
                        code: codes::UNKNOWN_RESOURCE,
                        severity: Severity::Warning,
                        message: format!(
                            "module {name:?}: step {:?} requires unknown resource \
                             {resource:?} (no mutual exclusion will apply)",
                            step.id
                        ),
                        labels: vec![Label {
                            span: step.span.clone().or_else(|| module.span.clone()),
                            note: String::new(),
                        }],
                        help: Some(format!("known: {}", KNOWN_RESOURCES.join(", "))),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let codes: Vec<_> = diagnostics.iter().map(|d| d.code).collect();
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
        assert!(check(&dup)
            .unwrap_err()
            .iter()
            .any(|d| d.code == codes::DUPLICATE_STEP));

        let reserved = STEPPED.replace(r#""id": "patch""#, r#""id": "done""#);
        assert!(check(&reserved)
            .unwrap_err()
            .iter()
            .any(|d| d.code == codes::DUPLICATE_STEP));
    }

    #[test]
    fn unknown_resource_warns_but_passes() {
        let json = STEPPED.replace(
            r#""needs": ["fetch"]"#,
            r#""needs": ["fetch"], "resources": ["my-lock"]"#,
        );
        // warnings don't fail the check
        let ir = check(&json).unwrap();
        assert!(ir.modules["helix"].steps.is_some());
        let diagnostics = validate(&ir);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == codes::UNKNOWN_RESOURCE && d.severity == Severity::Warning));
    }

    #[test]
    fn spanless_nodes_still_render() {
        let d = Diagnostic::error(codes::UNKNOWN_DEPENDENCY, "no span here")
            .with_label(None, "module xyz");
        let rendered = d.to_string();
        assert!(rendered.contains("= module xyz"));
    }
}
