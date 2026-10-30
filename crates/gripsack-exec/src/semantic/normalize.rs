//! Field equality fingerprints are review evidence, not a replacement recipe algorithm.
//! Ordered arrays retain their positions. Only contract-defined sets are sorted.
use super::model::*;
use gripsack_ir::{Span, workspace_model::identity::DefinitionDigest};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) fn digest(bytes: &[u8]) -> DefinitionDigest {
    let mut hash = Sha256::new();
    hash.update(b"gripsack/semantic-review/field/v1");
    hash.update(gripsack_policy::semantic::length_header(bytes.len() as u64));
    hash.update(bytes);
    DefinitionDigest::from_bytes(hash.finalize().into())
}
pub(super) fn escaped(text: &str) -> String {
    let mut result = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"_-.".contains(&byte) {
            result.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(result, "%{byte:02X}");
        }
    }
    result
}
pub(super) fn subject(namespace: &str, name: &str) -> SubjectRef {
    SubjectRef(format!("{namespace}/{}", escaped(name)))
}
pub(super) fn provenance(span: &Span) -> Provenance {
    let safe = !span.file.starts_with('/')
        && !span.file.contains("://")
        && !span.file.contains('@')
        && !span.file.chars().any(char::is_control)
        && span.file.len() <= MAX_TEXT_BYTES;
    Provenance {
        source: if span.file.contains("://") || span.file.contains('@') {
            digest(b"redacted-source-location")
        } else {
            digest(span.file.as_bytes())
        },
        source_label: safe.then(|| {
            span.file
                .split('/')
                .map(escaped)
                .collect::<Vec<_>>()
                .join("/")
        }),
        line: span.line,
        col: span.col,
    }
}
pub(super) fn unknown(path: &str, kind: ChangeKind, reason: UnknownReason) -> Fact {
    Fact {
        path: FieldPath(path.into()),
        kind,
        knowledge: KnowledgeState::Unknown { reason },
        origin: EvidenceOrigin::Uncaptured,
        observed_at: None,
    }
}
pub(super) fn known(path: &str, kind: ChangeKind, value: &[u8], origin: EvidenceOrigin) -> Fact {
    Fact {
        path: FieldPath(path.into()),
        kind,
        knowledge: KnowledgeState::Known {
            digest: digest(value),
        },
        origin,
        observed_at: None,
    }
}

fn unordered(key: &str) -> bool {
    matches!(
        key,
        "deps"
            | "runtime"
            | "checks"
            | "mutation_locks"
            | "include"
            | "exclude"
            | "tags"
            | "schedules"
            | "hooks"
            | "services"
            | "depends_on"
    )
}
fn canonicalize(value: &mut Value, key: &str) {
    match value {
        Value::Object(fields) => {
            fields.remove("span");
            // Line maps are diagnostic provenance, not executable script bytes.
            fields.remove("line_map");
            for (key, value) in fields {
                canonicalize(value, key);
            }
        }
        Value::Array(values) => {
            for value in values.iter_mut() {
                canonicalize(value, "");
            }
            if unordered(key) {
                // JSON Values have no Ord; decorate once, never serialize per comparison.
                let mut entries: Vec<_> = std::mem::take(values)
                    .into_iter()
                    .map(|value| (value.to_string(), value))
                    .collect();
                entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                *values = entries.into_iter().map(|(_, value)| value).collect();
            }
        }
        _ => {}
    }
}
fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["secret", "password", "token", "credential", "private_key"]
        .iter()
        .any(|part| key.contains(part))
        || matches!(
            key.as_str(),
            "vars" | "content" | "template" | "inline" | "payload" | "decrypted"
        )
}
fn secret_string(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("-----begin")
        || lower.contains("password=")
        || lower.contains("token=")
        || lower.contains("secret=")
        || (value.contains("://") && (value.contains('@') || value.contains('?')))
}
fn category(path: &str, output: &str, fallback: ChangeKind) -> ChangeKind {
    let field = path.split('/').nth(2).unwrap_or("");
    if matches!(
        field,
        "target"
            | "execution"
            | "host_runtime"
            | "policy"
            | "scope"
            | "trust_revision"
            | "publisher"
            | "key_id"
            | "namespace"
    ) {
        return ChangeKind::Eligibility;
    }
    if path
        .split('/')
        .any(|field| matches!(field, "checks" | "subject" | "stage"))
    {
        return ChangeKind::Validation;
    }
    if matches!(field, "layout" | "archive_root" | "materialization") {
        return ChangeKind::Materialization;
    }
    if field == "runtime" {
        return ChangeKind::Environment;
    }
    match output {
        "environment" => ChangeKind::Environment,
        "profile" => ChangeKind::Deployment,
        "task" | "hook" => ChangeKind::Invocation,
        "schedule" => ChangeKind::Registration,
        "check" => ChangeKind::Validation,
        "service" => ChangeKind::Invocation,
        _ => fallback,
    }
}
pub(super) fn facts(
    value: Value,
    prefix: &str,
    output: &str,
    fallback: ChangeKind,
    origin: EvidenceOrigin,
) -> Vec<Fact> {
    let mut value = value;
    canonicalize(&mut value, "");
    let mut result = Vec::new();
    flatten(&value, prefix, output, fallback, origin, &mut result, false);
    result
}
fn flatten(
    value: &Value,
    path: &str,
    output: &str,
    fallback: ChangeKind,
    origin: EvidenceOrigin,
    result: &mut Vec<Fact>,
    private: bool,
) {
    let kind = category(path, output, fallback);
    let key = path.rsplit('/').next().unwrap_or("");
    let private = private || sensitive_key(key) || value.as_str().is_some_and(secret_string);
    // Environment literal values are never persisted, including service credentials.
    let private =
        private || (path.contains("/env/") && !path.contains("/env/PATH/") && key == "value");
    if private {
        result.push(unknown(path, kind, UnknownReason::Redacted));
        return;
    }
    match value {
        Value::Object(fields)
            if fields.get("kind").and_then(Value::as_str) == Some("inherited") =>
        {
            result.push(unknown(path, kind, UnknownReason::MutableExternalState));
            if let Some(name) = fields.get("name") {
                result.push(known(
                    &format!("{path}/name"),
                    kind,
                    name.to_string().as_bytes(),
                    origin,
                ));
            }
        }
        Value::Object(fields) => {
            result.push(known(path, kind, b"object", origin));
            for (key, value) in fields {
                flatten(
                    value,
                    &format!("{path}/{}", escaped(key)),
                    output,
                    fallback,
                    origin,
                    result,
                    false,
                );
            }
        }
        Value::Array(values) => {
            result.push(known(
                path,
                kind,
                format!("array:{}", values.len()).as_bytes(),
                origin,
            ));
            for (index, value) in values.iter().enumerate() {
                flatten(
                    value,
                    &format!("{path}/{index}"),
                    output,
                    fallback,
                    origin,
                    result,
                    false,
                );
            }
        }
        _ => result.push(known(path, kind, value.to_string().as_bytes(), origin)),
    }
}
