//! Strict current authoring. Stored generations and receipts have their own readers.

use crate::{Diagnostic, Ir, codes};

pub const IR_VERSION: u32 = 10;

pub(crate) fn version_error(version: u64) -> Diagnostic {
    Diagnostic::error(
        codes::VERSION,
        format!("unsupported ir_version {version} (this core accepts {IR_VERSION})"),
    )
    .with_help("use the embedded frontend, or install the matching @gripsack/core SDK and declare a workspace in gripsack.ts")
}

/// Tagged-field admission runs before serde can discard unknown enum fields.
/// Move admitted values into the model; re-decode only errors for JSON locations.
pub fn parse(json: &str) -> Result<Ir, Diagnostic> {
    let ir = decode(json)?;
    crate::authoring::admit(&ir)?;
    Ok(ir)
}

fn decode(json: &str) -> Result<Ir, Diagnostic> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| {
        Diagnostic::error(codes::MALFORMED, format!("invalid IR JSON: {error}"))
    })?;
    match value.get("ir_version").and_then(serde_json::Value::as_u64) {
        Some(version) if version == u64::from(IR_VERSION) => decode_value(value, json),
        Some(version) => Err(version_error(version)),
        None => Err(Diagnostic::error(
            codes::MALFORMED,
            "missing or invalid ir_version",
        )),
    }
}

fn decode_value(value: serde_json::Value, json: &str) -> Result<Ir, Diagnostic> {
    let mut diagnostics = Vec::new();
    crate::tagged::check(&value, &mut diagnostics);
    if let Some(diagnostic) = diagnostics.into_iter().next() {
        return Err(diagnostic);
    }
    serde_json::from_value(value)
        .or_else(|_| serde_json::from_str(json))
        .map_err(|error| {
            Diagnostic::error(codes::MALFORMED, format!("invalid workspace IR: {error}"))
        })
}
