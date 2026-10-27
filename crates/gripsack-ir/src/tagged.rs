//! Close every tagged workspace union before serde can discard extra fields.

use crate::{Diagnostic, Span, codes};
use serde_json::Value;

mod workspace;

pub(crate) fn allowed_fetch_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "github_release" => &["kind", "repo", "asset", "version", "sha256", "base_url"],
        "tarball" => &["kind", "url", "sha256", "api_url"],
        "git" => &["kind", "url", "rev"],
        "file" => &["kind", "path"],
        "plugin" => &["kind", "name", "args"],
        _ => return None,
    })
}

fn check_tagged_at(
    node: &Value,
    path: &str,
    allowed: fn(&str) -> Option<&'static [&'static str]>,
    span: Option<&Span>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(object) = node.as_object() else {
        return;
    };
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        return;
    };
    let Some(fields) = allowed(kind) else { return };
    for key in object.keys() {
        if !fields.contains(&key.as_str()) {
            out.push(
                Diagnostic::error(
                    codes::MALFORMED,
                    format!("unknown field `{key}` in a {kind} node ({path})"),
                )
                .with_label(span.cloned(), "declared here"),
            );
        }
    }
}

/// Malformed or absent spans fall back to their enclosing declaration.
fn raw_node_span(node: &Value) -> Option<Span> {
    node.get("span")
        .and_then(|span| serde_json::from_value(span.clone()).ok())
}

pub(crate) fn check(value: &Value, out: &mut Vec<Diagnostic>) {
    workspace::check(value, out);
}
