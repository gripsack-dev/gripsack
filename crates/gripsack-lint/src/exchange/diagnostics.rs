//! Diagnostic coercion is protocol policy, independent of process supervision.

use gripsack_ir::{Diagnostic, Severity, Span};

const CRASH_CODES: [&str; 2] = ["E99", "E02"];

pub(super) fn from_plugin(
    raw: &serde_json::Value,
    module: &str,
    module_span: &Option<Span>,
) -> Diagnostic {
    let code = raw
        .get("code")
        .and_then(|c| c.as_str())
        .unwrap_or("griplint/?")
        .to_owned();
    let crash_class = CRASH_CODES
        .iter()
        .any(|c| code.rsplit('/').next() == Some(c));
    let severity = if !crash_class
        && raw
            .get("severity")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("error"))
    {
        Severity::Error
    } else {
        Severity::Warning
    };
    let mut labels = Vec::new();
    if let Some(raw_labels) = raw.get("labels").and_then(|l| l.as_array()) {
        for label in raw_labels {
            labels.push(gripsack_ir::Label {
                span: label
                    .get("span")
                    .and_then(|s| serde_json::from_value::<Span>(s.clone()).ok()),
                note: label
                    .get("note")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_owned(),
            });
        }
    }
    if labels.is_empty()
        && let Some(span) = module_span
    {
        labels.push(gripsack_ir::Label {
            span: Some(span.clone()),
            note: format!("module {module:?} requested this lint"),
        });
    }
    Diagnostic {
        code: std::borrow::Cow::Owned(code),
        severity,
        message: raw
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("(no message)")
            .to_owned(),
        labels,
        help: raw.get("help").and_then(|h| h.as_str()).map(str::to_owned),
    }
}

pub(super) fn host_diagnostic(
    name: &str,
    code: &str,
    severity: Severity,
    message: String,
    module: &str,
    module_span: &Option<Span>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic {
        code: std::borrow::Cow::Owned(format!("griplint-{name}/{code}")),
        severity,
        message,
        labels: Vec::new(),
        help: None,
    };
    if let Some(span) = module_span {
        diagnostic = diagnostic.with_label(
            Some(span.clone()),
            format!("module {module:?} requested this lint"),
        );
    }
    diagnostic
}
