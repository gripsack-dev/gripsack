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
        Diagnostic::error(codes::MALFORMED, format!("invalid IR JSON: {e}"))
            .with_help("the frontend emitted malformed IR — this is a frontend bug")
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
