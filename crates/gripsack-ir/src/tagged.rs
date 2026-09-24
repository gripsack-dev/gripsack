//! Pass 1.5 — tagged-field validation (the contract, load-bearing):
//! internally-tagged enums (fetch, build, activate, verify, step
//! actions) can't use serde's deny_unknown_fields, so unknown fields
//! inside a tagged node are checked by hand against per-kind
//! allowlists. A leak like `baseUrl` (the TS frontend's, silently
//! dropped for months) is a hard error, never silent data loss (0009,
//! review finding B). Every nested tagged node is closed: a step's
//! fetch spec or verify check gets the same admission as a
//! module-level one.

use crate::diagnostic::{Diagnostic, codes};

mod workspace;

/// kind → allowed keys (the tag itself plus the variant's fields).
fn allowed_fetch_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "github_release" => &["kind", "repo", "asset", "version", "sha256", "base_url"],
        "tarball" => &["kind", "url", "sha256", "api_url"],
        "git" => &["kind", "url", "rev"],
        "file" => &["kind", "path"],
        "plugin" => &["kind", "name", "args"],
        "brew" => &["kind", "formula", "version", "sha256"],
        "pixi" => &["kind", "package", "version", "sha256"],
        _ => return None,
    })
}

fn allowed_build_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "none" => &["kind"],
        "custom_shell" => &["kind", "script"],
        _ => return None,
    })
}

/// Activation actions — both standalone intents (where `trigger` and
/// `span` sit beside the flattened action fields) and the `intent`
/// step action's inner node.
fn allowed_intent_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "service" => &["kind", "name", "user", "trigger", "span"],
        "fonts" | "desktop_entry" => &["kind", "trigger", "span"],
        "custom_shell" => &["kind", "script", "trigger", "span"],
        _ => return None,
    })
}

/// The same actions without the intent envelope keys.
fn allowed_action_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "service" => &["kind", "name", "user"],
        "fonts" | "desktop_entry" => &["kind"],
        "custom_shell" => &["kind", "script"],
        _ => return None,
    })
}

fn allowed_verify_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "binary_runs" => &["kind", "path", "args"],
        "file_exists" | "file_deployed" => &["kind", "path"],
        "shell" => &["kind", "script"],
        _ => return None,
    })
}

fn allowed_step_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "fetch" => &[
            "kind",
            "fetch",
            "needs",
            "resources",
            "verify",
            "span",
            "phase",
        ],
        "build" => &[
            "kind",
            "spec",
            "needs",
            "resources",
            "verify",
            "span",
            "phase",
        ],
        "install" | "config_deploy" => &[
            "kind",
            "entries",
            "needs",
            "resources",
            "verify",
            "span",
            "phase",
        ],
        "intent" => &[
            "kind",
            "action",
            "trigger",
            "needs",
            "resources",
            "verify",
            "span",
            "phase",
        ],
        "verify" => &[
            "kind",
            "verify",
            "needs",
            "resources",
            "verify",
            "span",
            "phase",
        ],
        "run" => &[
            "kind",
            "argv",
            "env",
            "cwd",
            "outputs",
            "needs",
            "resources",
            "verify",
            "span",
            "phase",
        ],
        "custom_shell" => &[
            "kind",
            "script",
            "outputs",
            "needs",
            "resources",
            "verify",
            "span",
            "phase",
        ],
        _ => return None,
    })
}

fn check_tagged(
    node: &serde_json::Value,
    path: &str,
    allowed: fn(&str) -> Option<&'static [&'static str]>,
    out: &mut Vec<Diagnostic>,
) {
    check_tagged_at(node, path, allowed, None, out);
}

/// `span` labels the rejection at the nearest declaring node. The v4
/// workspace walk passes it (v4 spans are mandatory; A1-06 requires
/// source-carrying rejection); the v3 module walk passes None and keeps
/// its span-free message — v3 behavior preserved.
fn check_tagged_at(
    node: &serde_json::Value,
    path: &str,
    allowed: fn(&str) -> Option<&'static [&'static str]>,
    span: Option<&crate::Span>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(obj) = node.as_object() else { return };
    let Some(kind) = obj.get("kind").and_then(|k| k.as_str()) else {
        return;
    };
    if let Some(fields) = allowed(kind) {
        for key in obj.keys() {
            if !fields.contains(&key.as_str()) {
                let diagnostic = Diagnostic::error(
                    codes::MALFORMED,
                    format!("unknown field `{key}` in a {kind} node ({path})"),
                );
                out.push(match span {
                    Some(span) => diagnostic.with_label(Some(span.clone()), "declared here"),
                    None => diagnostic,
                });
            }
        }
    }
}

/// A v4 workspace node's own `span`, read from the raw JSON before
/// serde. None when absent or malformed — the caller falls back to the
/// enclosing node's span.
fn raw_node_span(node: &serde_json::Value) -> Option<crate::Span> {
    node.get("span")
        .and_then(|s| serde_json::from_value::<crate::Span>(s.clone()).ok())
}

/// Walk the IR JSON, validating every tagged node. Runs at parse time,
/// before serde drops unknown fields — the pass order matters. Also owns
/// the version-dispatched envelope shape (0052 §2.1): serde cannot
/// express the v4 cross-key exclusivity, so it is admitted here on raw
/// keys before deserialization.
pub fn tagged_field_check(json: &str, out: &mut Vec<Diagnostic>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return; // pass 1 reports the syntax error itself
    };
    let version = value.get("ir_version").and_then(|v| v.as_u64());
    if let Some(version) = version {
        if !(u64::from(crate::parse::LEGACY_IR_VERSION)..=u64::from(crate::parse::IR_VERSION))
            .contains(&version)
        {
            out.push(Diagnostic::error(
                codes::VERSION,
                format!(
                    "unsupported ir_version {version} (this core accepts {}..={})",
                    crate::parse::LEGACY_IR_VERSION,
                    crate::parse::IR_VERSION
                ),
            ).with_help("update a pinned @gripsack/core to a release that emits ir_version 4, or remove the pin to use the embedded frontend"));
            return;
        }
        if let Some(root) = value.as_object() {
            envelope_check(root, version, out);
        }
    }
    check_modules_tagged(&value, out);
    if version == Some(u64::from(crate::parse::IR_VERSION)) {
        workspace::check(&value, out);
    }
}

/// The v3/v4 envelope split (plan/0052 §2.1, schema/ir/v4.json root
/// `oneOf`): v3 is the strict module map — required `modules`, no
/// `workspace`, optional `host`; v4 requires core-injected `host` facts
/// and exactly one of `workspace` or legacy `modules`.
fn envelope_check(
    root: &serde_json::Map<String, serde_json::Value>,
    version: u64,
    out: &mut Vec<Diagnostic>,
) {
    let has_workspace = root.contains_key("workspace");
    let has_modules = root.contains_key("modules");
    if version == u64::from(crate::parse::LEGACY_IR_VERSION) {
        if has_workspace {
            out.push(Diagnostic::error(
                codes::MALFORMED,
                "`workspace` requires ir_version 4; an ir_version 3 envelope declares a module map",
            )
            .with_help("emit ir_version 4 for workspace declarations, or remove the workspace key"));
        }
        if !has_modules {
            out.push(Diagnostic::error(
                codes::MALFORMED,
                "missing field `modules`: an ir_version 3 envelope declares a module map",
            ));
        }
    } else if version == u64::from(crate::parse::IR_VERSION) {
        if !root.contains_key("host") {
            out.push(Diagnostic::error(
                codes::MALFORMED,
                "ir_version 4 requires core-injected `host` facts (schema/ir/v4.json)",
            ));
        }
        match (has_workspace, has_modules) {
            (true, true) => out.push(Diagnostic::error(
                codes::MALFORMED,
                "ir_version 4 envelope declares both `workspace` and `modules`; exactly one is allowed",
            )),
            (false, false) => out.push(Diagnostic::error(
                codes::MALFORMED,
                "ir_version 4 envelope declares neither `workspace` nor `modules`; exactly one is required",
            )),
            _ => {}
        }
    }
}

/// The v3 module map — walked for ir_version 3 and for the v4
/// legacy-modules compatibility branch alike.
fn check_modules_tagged(value: &serde_json::Value, out: &mut Vec<Diagnostic>) {
    let Some(modules) = value.get("modules").and_then(|m| m.as_object()) else {
        return;
    };
    for (name, module) in modules {
        let path = format!("module {name:?}");
        // Reject invalid dependency purposes before enum deserialization so
        // E122 can retain the dependency's declaration span.
        if let Some(deps) = module.get("depends").and_then(|d| d.as_array()) {
            for dep in deps {
                if let Some(edge) = dep.get("for")
                    && !matches!(edge.as_str(), Some("runtime" | "build"))
                {
                    let span = dep
                        .get("span")
                        .or_else(|| module.get("span"))
                        .and_then(|s| serde_json::from_value::<crate::Span>(s.clone()).ok());
                    out.push(Diagnostic::error(
                        codes::UNKNOWN_EDGE,
                        format!("{path}: unknown dependency purpose `for: {edge}`; expected \"runtime\" or \"build\""),
                    ).with_label(span, "dependency declared here"));
                }
            }
        }
        if let Some(fetch) = module.get("fetch") {
            check_tagged(fetch, &path, allowed_fetch_fields, out);
        }
        if let Some(build) = module.get("build") {
            check_tagged(build, &path, allowed_build_fields, out);
        }
        if let Some(activate) = module.get("activate").and_then(|a| a.as_array()) {
            for intent in activate {
                check_tagged(intent, &path, allowed_intent_fields, out);
            }
        }
        if let Some(verify) = module.get("verify") {
            check_tagged(verify, &path, allowed_verify_fields, out);
        }
        if let Some(steps) = module.get("steps").and_then(|s| s.as_array()) {
            for step in steps {
                let id = step
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("?")
                    .to_string();
                let kind = step
                    .get("action")
                    .and_then(|a| a.get("kind"))
                    .and_then(|k| k.as_str())
                    .unwrap_or("")
                    .to_string();
                let step_path = format!("{path} step {id:?} ({kind})");
                // step-level keys (id/action/phase/needs/resources/...)
                const STEP_KEYS: &[&str] = &[
                    "id",
                    "action",
                    "phase",
                    "needs",
                    "resources",
                    "verify",
                    "span",
                ];
                if let Some(obj) = step.as_object() {
                    for key in obj.keys() {
                        if !STEP_KEYS.contains(&key.as_str()) {
                            out.push(Diagnostic::error(
                                codes::MALFORMED,
                                format!("unknown field `{key}` in {step_path}"),
                            ));
                        }
                    }
                }
                if let Some(verify) = step.get("verify") {
                    check_tagged(verify, &step_path, allowed_verify_fields, out);
                }
                if let Some(action) = step.get("action") {
                    check_tagged(action, &step_path, allowed_step_fields, out);
                    // Nested tagged nodes inside the action get the same
                    // admission as their module-level twins.
                    match kind.as_str() {
                        "fetch" => {
                            if let Some(fetch) = action.get("fetch") {
                                check_tagged(fetch, &step_path, allowed_fetch_fields, out);
                            }
                        }
                        "build" => {
                            if let Some(spec) = action.get("spec") {
                                check_tagged(spec, &step_path, allowed_build_fields, out);
                            }
                        }
                        "verify" => {
                            if let Some(verify) = action.get("verify") {
                                check_tagged(verify, &step_path, allowed_verify_fields, out);
                            }
                        }
                        "intent" => {
                            if let Some(inner) = action.get("action") {
                                check_tagged(inner, &step_path, allowed_action_fields, out);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
