//! Pass 1.5 — tagged-field validation (the contract, load-bearing):
//! internally-tagged enums (fetch/steps/activate) can't use serde's
//! deny_unknown_fields, so unknown fields inside a tagged node are
//! checked by hand against per-kind allowlists. A leak like `baseUrl`
//! (the TS frontend's, silently dropped for months) is a hard error,
//! never silent data loss (0009, review finding B).

use crate::diagnostic::{Diagnostic, codes};

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

fn allowed_step_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "fetch" => &[
            "kind",
            "fetch",
            "needs",
            "resources",
            "verify",
            "retries",
            "span",
            "phase",
        ],
        "build" => &[
            "kind",
            "spec",
            "needs",
            "resources",
            "verify",
            "retries",
            "span",
            "phase",
        ],
        "install" | "config_deploy" => &[
            "kind",
            "entries",
            "needs",
            "resources",
            "verify",
            "retries",
            "span",
            "phase",
        ],
        "intent" => &[
            "kind",
            "action",
            "needs",
            "resources",
            "verify",
            "retries",
            "span",
            "phase",
        ],
        "verify" => &[
            "kind",
            "verify",
            "needs",
            "resources",
            "verify",
            "retries",
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
            "retries",
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
            "retries",
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
    let Some(obj) = node.as_object() else { return };
    let Some(kind) = obj.get("kind").and_then(|k| k.as_str()) else {
        return;
    };
    if let Some(fields) = allowed(kind) {
        for key in obj.keys() {
            if !fields.contains(&key.as_str()) {
                out.push(Diagnostic::error(
                    codes::MALFORMED,
                    format!("unknown field `{key}` in a {kind} node ({path})"),
                ));
            }
        }
    }
}

/// Walk the IR JSON, validating every tagged node. Runs at parse time,
/// before serde drops unknown fields — the pass order matters.
pub fn tagged_field_check(json: &str, out: &mut Vec<Diagnostic>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return; // pass 1 reports the syntax error itself
    };
    let Some(modules) = value.get("modules").and_then(|m| m.as_object()) else {
        return;
    };
    for (name, module) in modules {
        let path = format!("module {name:?}");
        if let Some(fetch) = module.get("fetch") {
            check_tagged(fetch, &path, allowed_fetch_fields, out);
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
                    "retries",
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
                if let Some(action) = step.get("action") {
                    check_tagged(action, &step_path, allowed_step_fields, out);
                }
            }
        }
    }
}
