//! Pass 1 — parse (0004 §4): syntax + version gate.

use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;

/// The only IR version this core accepts (for now).
pub const IR_VERSION: u32 = 1;

/// Parse IR JSON into the typed model (E000 malformed, E100 version).
/// Pass 1.5 (tagged-field validation) runs BEFORE serde drops unknown
/// fields — a leak is a hard error, never silent data loss.
pub fn parse(json: &str) -> Result<Ir, Diagnostic> {
    let mut tagged_diagnostics = Vec::new();
    crate::tagged::tagged_field_check(json, &mut tagged_diagnostics);
    if let Some(d) = tagged_diagnostics.into_iter().next() {
        return Err(d);
    }
    let ir: Ir = serde_json::from_str(json).map_err(|e| {
        // A typed failure deep in serialized IR names a byte offset,
        // nothing else — on a 40-module graph that is not a location
        // anyone can act on, and "this is a frontend bug" is usually
        // wrong: the common cause is a user-side type slip (an
        // argument in the wrong order, a stale @gripsack/core pin
        // typechecking an old call shape). Re-walk the modules
        // individually to name the culprits (migration report
        // 0.18.1).
        let detail = match name_failing_modules(json) {
            Some(names) => format!("invalid IR in module(s) {names}: {e}"),
            None => format!("invalid IR JSON: {e}"),
        };
        Diagnostic::error(codes::MALFORMED, detail).with_help(
            "a field has the wrong type — check the named module's call sites and \
             your package.json @gripsack/core pin (grip doctor compares it); if \
             both are current, this is a frontend bug",
        )
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

/// Deserialize each module individually and name the ones that fail —
/// attribution the byte-offset serde error cannot give. None when the
/// JSON is not even shaped like an IR envelope (the whole-document
/// error stands on its own).
fn name_failing_modules(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let modules = value.get("modules")?.as_object()?;
    let mut failed: Vec<String> = Vec::new();
    let mut first_error = None;
    for (name, module) in modules {
        if let Err(e) = serde_json::from_value::<crate::model::Module>(module.clone()) {
            if first_error.is_none() {
                first_error = Some(e.to_string());
            }
            failed.push(format!("{name:?}"));
        }
    }
    if failed.is_empty() {
        None
    } else {
        // cap the list: naming three is actionable, naming forty is noise
        let mut names = failed.join(", ");
        if failed.len() > 3 {
            names = format!("{}, and {} more", names, failed.len() - 3);
        }
        Some(names)
    }
}
