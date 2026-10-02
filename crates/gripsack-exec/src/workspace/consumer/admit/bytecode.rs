//! A frozen materializer policy also constrains native execution. The same
//! suppression is installed in environment overlays and durable command wrappers.
use super::{ExecError, Span, gate};
use gripsack_ir::workspace_v6::lock::BytecodePolicy;
use std::ffi::OsString;

pub(in crate::workspace::consumer) fn apply(
    policy: Option<BytecodePolicy>,
    entries: &mut Vec<(OsString, OsString)>,
    span: &Span,
) -> Result<(), ExecError> {
    match policy {
        None => Ok(()),
        Some(BytecodePolicy::Suppress) => {
            const KEY: &str = "PYTHONDONTWRITEBYTECODE";
            if entries
                .iter()
                .any(|(name, value)| name == KEY && value != "1")
            {
                return Err(gate(
                    span,
                    "PYTHONDONTWRITEBYTECODE conflicts with the frozen Conda suppress-bytecode policy",
                ));
            }
            if !entries.iter().any(|(name, _)| name == KEY) {
                entries.push((KEY.into(), "1".into()));
            }
            Ok(())
        }
    }
}
