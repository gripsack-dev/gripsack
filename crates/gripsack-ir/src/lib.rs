//! IR v1 (draft) — the contract between frontends and the core.
//! See plan/0001 §3.2, plan/0004 (spans, diagnostics, passes) and
//! schema/ir/v1.json. Change all three sides together
//! (`.agents/skills/gripsack-ir`).

pub mod diagnostic;
pub mod model;
pub mod parse;
pub mod sema;
pub mod span;
pub mod step;

pub use diagnostic::{codes, Diagnostic, Label, Severity};
pub use model::*;
pub use parse::{parse, IR_VERSION};
/// Backwards-compatible alias: pass 2 is `sema::run`.
pub use sema::run as validate;
pub use sema::{check, run};
pub use span::Span;
pub use step::*;
