//! IR8 additions. Historical allowlists remain closed and unchanged.

pub(super) fn output_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "package" => Some(&[
            "kind",
            "name",
            "span",
            "producer",
            "commands",
            "runtime",
            "target",
            "layout",
            "host_runtime",
            "archive_root",
        ]),
        "profile" => Some(&[
            "kind",
            "name",
            "span",
            "files",
            "environment",
            "schedules",
            "hooks",
            "launchers",
        ]),
        _ => super::v6::output_fields_v7(kind),
    }
}

pub(super) fn source_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "repo_tree" => Some(&["kind", "path", "include", "exclude"]),
        _ => super::v6::source_fields(kind),
    }
}

pub(super) fn content_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "template_file" => Some(&["kind", "variables", "result_digest"]),
        _ => super::v6::content_fields(kind),
    }
}

pub(super) fn archive_root_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "single_directory" => Some(&["kind"]),
        _ => None,
    }
}

pub(super) fn reject_historical_variant(
    node: &serde_json::Value,
    path: &str,
    span: Option<&crate::Span>,
    out: &mut Vec<crate::Diagnostic>,
) {
    if let Some(kind @ ("repo_tree" | "template_file")) =
        node.get("kind").and_then(serde_json::Value::as_str)
    {
        out.push(
            crate::Diagnostic::error(
                crate::codes::MALFORMED,
                format!("{kind} requires ir_version 8 ({path})"),
            )
            .with_label(span.cloned(), "declared here"),
        );
    }
}

pub(super) fn launchers(
    node: &serde_json::Value,
    path: &str,
    span: Option<&crate::Span>,
    out: &mut Vec<crate::Diagnostic>,
) {
    let Some(fields) = node.as_object() else {
        return;
    };
    for key in fields.keys() {
        if !matches!(key.as_str(), "directory" | "commands") {
            out.push(
                crate::Diagnostic::error(
                    crate::codes::MALFORMED,
                    format!("unknown field `{key}` in profile launchers ({path})"),
                )
                .with_label(span.cloned(), "declared here"),
            );
        }
    }
}
