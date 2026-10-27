//! Parse one current workspace, rejecting obsolete versions before decoding.

use crate::{Diagnostic, Ir, codes};

pub const IR_VERSION: u32 = 9;

pub(crate) fn version_error(version: u64) -> Diagnostic {
    Diagnostic::error(
        codes::VERSION,
        format!("unsupported ir_version {version} (this core accepts {IR_VERSION})"),
    )
    .with_help("use the embedded frontend, or install the matching @gripsack/core SDK and declare a workspace in gripsack.ts")
}

/// Tagged-field admission runs before serde can discard unknown enum fields.
/// Decode the JSON once; move the admitted value into the current typed model.
pub fn parse(json: &str) -> Result<Ir, Diagnostic> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| {
        Diagnostic::error(codes::MALFORMED, format!("invalid IR JSON: {error}"))
    })?;
    if let Some(version) = value.get("ir_version").and_then(serde_json::Value::as_u64)
        && version != u64::from(IR_VERSION)
    {
        return Err(version_error(version));
    }
    let mut diagnostics = Vec::new();
    crate::tagged::check(&value, &mut diagnostics);
    if let Some(diagnostic) = diagnostics.into_iter().next() {
        return Err(diagnostic);
    }
    serde_json::from_value(value).map_err(|error| {
        Diagnostic::error(codes::MALFORMED, format!("invalid workspace IR: {error}"))
    })
}
