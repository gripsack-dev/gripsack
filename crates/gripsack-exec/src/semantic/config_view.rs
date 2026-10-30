//! Read-only explanations of exact proposed bytes, never an operation planner.
//!
//! Equality is only JSON/TOML parser-value equality, not application equivalence.
//! Parsers own admission and byte spans; this module owns comparison and disclosure.
//! No raw input, value digest, parser diagnostic, or execution authority is serialized.

use std::collections::BTreeMap;

use serde::Serialize;

#[path = "config_view/json.rs"]
mod json;
#[cfg(test)]
#[path = "config_view/tests.rs"]
mod tests;
#[path = "config_view/toml.rs"]
mod toml;

// Review-only resource policy; these limits never change deployed bytes.
pub const MAX_INPUT_BYTES: usize = 256 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 8192;
const MAX_CHANGES: usize = 256;
const MAX_PATH_BYTES: usize = 2048;
const MAX_VALUE_BYTES: usize = 256;
const MAX_BYTE_RANGES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFormat {
    Json,
    Toml,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigScope {
    WholeFile,
    ManagedBlock,
    ForeignObservation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValueVisibility {
    #[default]
    Portable,
    Local,
}

#[derive(Clone, Debug, Default)]
pub struct ConfigViewOptions {
    pub values: ValueVisibility,
    /// Exact bracket paths whose entire subtrees must be redacted in local views.
    pub secret_paths: Vec<String>,
}

impl ConfigViewOptions {
    pub fn local() -> Self {
        Self {
            values: ValueVisibility::Local,
            secret_paths: Vec::new(),
        }
    }
}

/// Half-open byte offsets into the supplied slice, not its containing file.
/// Structural spans are UTF-8 aligned; byte-fallback spans need not be.
/// TOML table spans select their parser declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl From<std::ops::Range<usize>> for SourceSpan {
    fn from(range: std::ops::Range<usize>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Null,
    Boolean,
    JsonNumber,
    Integer,
    Float,
    String,
    Datetime,
    Object,
    Array,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeReason {
    Added,
    Removed,
    ValueChanged,
    TypeChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueDisclosure {
    OmittedPortable,
    Redacted,
    Visible,
    ContainerSummary,
}

#[derive(Clone, Debug, Serialize)]
pub struct PresentedValue {
    pub kind: ValueKind,
    pub disclosure: ValueDisclosure,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldChange {
    /// JSON bracket notation: `$["escaped key"][0]`. Never terminal-active text.
    pub path: String,
    pub path_redacted: bool,
    pub reason: ChangeReason,
    pub before_span: Option<SourceSpan>,
    pub after_span: Option<SourceSpan>,
    pub before: Option<PresentedValue>,
    pub after: Option<PresentedValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    UnsupportedFormat,
    Oversized,
    Binary,
    InvalidSyntax,
    DuplicateKey,
    ResourceLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    StructuralViewUnavailable,
    ByteComparisonBounded,
    ChangesTruncated,
    ValuesTruncated,
}

#[derive(Clone, Debug, Serialize)]
pub struct ByteDifference {
    pub before_span: Option<SourceSpan>,
    pub after_span: Option<SourceSpan>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ByteFallback {
    pub before_reason: Option<FallbackReason>,
    pub after_reason: Option<FallbackReason>,
    pub before_bytes: usize,
    pub after_bytes: usize,
    /// Same-offset differing runs in bounded prefixes; not a minimal edit script.
    pub differences: Vec<ByteDifference>,
    pub inspected_before_bytes: usize,
    pub inspected_after_bytes: usize,
    pub truncated: bool,
    /// Fallback never discloses raw bytes, even in local mode: no parsed fields
    /// exist with which to apply secret-field policy safely.
    pub raw_bytes_omitted: bool,
    pub binary_omitted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConfigView {
    pub format: ConfigFormat,
    pub scope: ConfigScope,
    pub exact_bytes_equal: Option<bool>,
    pub structural_equal: Option<bool>,
    pub comparison_contract: &'static str,
    pub changes: Vec<FieldChange>,
    pub fallback: Option<ByteFallback>,
    /// Complete structural change coverage, independently of disclosure.
    pub complete: bool,
    pub redacted: bool,
    pub truncated: bool,
    /// The caller supplied exact bytes but no observation timestamp.
    pub evidence_age: &'static str,
    pub unknown_reasons: Vec<UnknownReason>,
}

#[derive(Debug)]
struct Node {
    span: SourceSpan,
    value: Value,
}

#[derive(Debug, PartialEq)]
enum Value {
    Null,
    Boolean(bool),
    JsonNumber(serde_json::Number),
    Integer(i64),
    Float(f64),
    String(String),
    Datetime(String),
    Object(BTreeMap<String, Node>),
    Array(Vec<Node>),
}

// Provenance is deliberately excluded from parser-value equality.
impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Node {
    fn kind(&self) -> ValueKind {
        match self.value {
            Value::Null => ValueKind::Null,
            Value::Boolean(_) => ValueKind::Boolean,
            Value::JsonNumber(_) => ValueKind::JsonNumber,
            Value::Integer(_) => ValueKind::Integer,
            Value::Float(_) => ValueKind::Float,
            Value::String(_) => ValueKind::String,
            Value::Datetime(_) => ValueKind::Datetime,
            Value::Object(_) => ValueKind::Object,
            Value::Array(_) => ValueKind::Array,
        }
    }
}

/// Compare borrowed exact bytes without reading files, executing tools, or
/// changing ownership decisions. Missing evidence must not be passed as empty
/// bytes: an empty slice is an observed empty file (and invalid JSON).
pub fn compare_config(
    before: &[u8],
    after: &[u8],
    format: ConfigFormat,
    scope: ConfigScope,
    options: &ConfigViewOptions,
) -> ConfigView {
    let before_prefix = &before[..before.len().min(MAX_INPUT_BYTES)];
    let after_prefix = &after[..after.len().min(MAX_INPUT_BYTES)];
    let exact_bytes_equal = if before.len() != after.len() || before_prefix != after_prefix {
        Some(false)
    } else if before.len() <= MAX_INPUT_BYTES {
        Some(true)
    } else {
        None
    };
    let mut view = ConfigView {
        format,
        scope,
        exact_bytes_equal,
        structural_equal: None,
        comparison_contract: match format {
            ConfigFormat::Json => "strict_json_serde_json_scalars_ordered_arrays",
            ConfigFormat::Toml => "toml_0.9_values_ordered_arrays_f64_partial_equality",
            ConfigFormat::Unsupported => "bounded_bytes_only",
        },
        changes: Vec::new(),
        fallback: None,
        complete: false,
        redacted: options.values == ValueVisibility::Portable,
        truncated: false,
        evidence_age: "not_supplied",
        unknown_reasons: Vec::new(),
    };
    match (parse(before, format), parse(after, format)) {
        (Ok(before), Ok(after)) => {
            view.structural_equal = Some(before == after);
            compare_nodes(Some(&before), Some(&after), "$", false, options, &mut view);
            view.complete = !view.truncated;
            if view.truncated {
                view.unknown_reasons.push(UnknownReason::ChangesTruncated);
            }
            if view.changes.iter().any(|change| {
                change
                    .before
                    .iter()
                    .chain(change.after.iter())
                    .any(|value| value.truncated)
            }) {
                view.truncated = true;
                view.unknown_reasons.push(UnknownReason::ValuesTruncated);
            }
        }
        (before_result, after_result) => {
            let fallback = byte_fallback(before, after, before_result.err(), after_result.err());
            view.truncated = fallback.truncated;
            view.redacted = true;
            view.fallback = Some(fallback);
            view.unknown_reasons
                .push(UnknownReason::StructuralViewUnavailable);
        }
    }
    if exact_bytes_equal.is_none() {
        view.unknown_reasons
            .push(UnknownReason::ByteComparisonBounded);
    }
    view
}

fn parse(bytes: &[u8], format: ConfigFormat) -> Result<Node, FallbackReason> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(FallbackReason::Oversized);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| FallbackReason::Binary)?;
    if text
        .chars()
        .any(|c| c.is_ascii_control() && !matches!(c, '\t' | '\r' | '\n'))
    {
        return Err(FallbackReason::Binary);
    }
    match format {
        ConfigFormat::Json => json::parse(text),
        ConfigFormat::Toml => toml::parse(text),
        ConfigFormat::Unsupported => Err(FallbackReason::UnsupportedFormat),
    }
}

fn compare_nodes(
    before: Option<&Node>,
    after: Option<&Node>,
    path: &str,
    secret: bool,
    options: &ConfigViewOptions,
    view: &mut ConfigView,
) {
    if before == after {
        return;
    }
    if view.changes.len() == MAX_CHANGES {
        view.truncated = true;
        return;
    }
    let secret = secret
        || options
            .secret_paths
            .iter()
            .any(|candidate| candidate == path);
    match (before.map(|n| &n.value), after.map(|n| &n.value)) {
        (Some(Value::Object(old)), Some(Value::Object(new))) => {
            for (key, value) in old {
                compare_nodes(
                    Some(value),
                    new.get(key),
                    &field_path(path, key),
                    secret || secret_key(key),
                    options,
                    view,
                );
            }
            for (key, value) in new {
                if !old.contains_key(key) {
                    compare_nodes(
                        None,
                        Some(value),
                        &field_path(path, key),
                        secret || secret_key(key),
                        options,
                        view,
                    );
                }
            }
            return;
        }
        (Some(Value::Array(old)), Some(Value::Array(new))) => {
            for index in 0..old.len().max(new.len()) {
                compare_nodes(
                    old.get(index),
                    new.get(index),
                    &format!("{path}[{index}]"),
                    secret,
                    options,
                    view,
                );
            }
            return;
        }
        _ => {}
    }
    let reason = match (before, after) {
        (None, _) => ChangeReason::Added,
        (_, None) => ChangeReason::Removed,
        (Some(old), Some(new)) if old.kind() != new.kind() => ChangeReason::TypeChanged,
        _ => ChangeReason::ValueChanged,
    };
    let before_value = before.map(|node| present(node, secret, options));
    let after_value = after.map(|node| present(node, secret, options));
    for value in before_value.iter().chain(after_value.iter()) {
        view.redacted |= matches!(
            value.disclosure,
            ValueDisclosure::Redacted | ValueDisclosure::OmittedPortable
        );
        // Value clipping does not truncate change coverage.
    }
    let path_redacted = sensitive_text(path);
    view.redacted |= path_redacted;
    view.changes.push(FieldChange {
        path: if path_redacted {
            "[redacted_path]".to_owned()
        } else {
            path.to_owned()
        },
        path_redacted,
        reason,
        before_span: before.map(|node| node.span),
        after_span: after.map(|node| node.span),
        before: before_value,
        after: after_value,
    });
}

fn field_path(parent: &str, key: &str) -> String {
    // JSON string escaping is injective; additionally escape Unicode format
    // controls that JSON itself permits but terminals can interpret visually.
    let mut encoded = String::new();
    for character in serde_json::to_string(key)
        .expect("string serialization cannot fail")
        .chars()
    {
        if character.is_control() || format_control(character) {
            use std::fmt::Write;
            write!(encoded, "\\u{:04x}", u32::from(character)).expect("string write cannot fail");
        } else {
            encoded.push(character);
        }
    }
    format!("{parent}[{encoded}]")
}

fn secret_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_', ' '], "");
    [
        "password",
        "passwd",
        "secret",
        "token",
        "credential",
        "privatekey",
        "apikey",
        "accesskey",
        "authorization",
        "cookie",
    ]
    .iter()
    .any(|word| normalized.contains(word))
}

fn format_control(character: char) -> bool {
    matches!(character, '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
}

fn sensitive_text(text: &str) -> bool {
    ["://", "-----begin", "bearer ", "password=", "token="]
        .iter()
        .any(|needle| {
            text.as_bytes()
                .windows(needle.len())
                .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
        })
}

fn present(node: &Node, secret: bool, options: &ConfigViewOptions) -> PresentedValue {
    let mut value = PresentedValue {
        kind: node.kind(),
        disclosure: ValueDisclosure::OmittedPortable,
        text: None,
        truncated: false,
    };
    if options.values == ValueVisibility::Portable {
        return value;
    }
    let suspicious = match &node.value {
        Value::String(text) => sensitive_text(text),
        _ => false,
    };
    if secret || suspicious {
        value.disclosure = ValueDisclosure::Redacted;
        return value;
    }
    let text = match &node.value {
        Value::String(text) | Value::Datetime(text) => text.as_str(),
        Value::Object(_) | Value::Array(_) => {
            value.disclosure = ValueDisclosure::ContainerSummary;
            return value;
        }
        _ => {
            value.disclosure = ValueDisclosure::Visible;
            value.text = Some(match &node.value {
                Value::Null => "null".to_owned(),
                Value::Boolean(v) => v.to_string(),
                Value::JsonNumber(v) => v.to_string(),
                Value::Integer(v) => v.to_string(),
                Value::Float(v) => v.to_string(),
                _ => unreachable!("containers and strings handled above"),
            });
            return value;
        }
    };
    let mut sanitized = String::new();
    for character in text.chars() {
        let escaped = character.is_control() || format_control(character) || character == '\\';
        let size = if escaped {
            character.escape_default().len()
        } else {
            character.len_utf8()
        };
        if sanitized.len() + size > MAX_VALUE_BYTES {
            value.truncated = true;
            break;
        }
        if escaped {
            sanitized.extend(character.escape_default());
        } else {
            sanitized.push(character);
        }
    }
    value.disclosure = ValueDisclosure::Visible;
    value.text = Some(sanitized);
    value
}

fn byte_fallback(
    before: &[u8],
    after: &[u8],
    before_reason: Option<FallbackReason>,
    after_reason: Option<FallbackReason>,
) -> ByteFallback {
    let old = &before[..before.len().min(MAX_INPUT_BYTES)];
    let new = &after[..after.len().min(MAX_INPUT_BYTES)];
    let mut differences = Vec::new();
    let mut index = 0;
    while index < old.len().max(new.len()) && differences.len() < MAX_BYTE_RANGES {
        if old.get(index) == new.get(index) {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < old.len().max(new.len()) && old.get(index) != new.get(index) {
            index += 1;
        }
        differences.push(ByteDifference {
            before_span: (start < old.len()).then_some(SourceSpan {
                start,
                end: index.min(old.len()),
            }),
            after_span: (start < new.len()).then_some(SourceSpan {
                start,
                end: index.min(new.len()),
            }),
        });
    }
    ByteFallback {
        before_reason,
        after_reason,
        before_bytes: before.len(),
        after_bytes: after.len(),
        differences,
        inspected_before_bytes: index.min(old.len()),
        inspected_after_bytes: index.min(new.len()),
        truncated: index < before.len().max(after.len()),
        raw_bytes_omitted: true,
        binary_omitted: before_reason == Some(FallbackReason::Binary)
            || after_reason == Some(FallbackReason::Binary),
    }
}
