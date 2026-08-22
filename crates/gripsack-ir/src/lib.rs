//! IR v1 (draft) — the contract between frontends and the core.
//! See plan/0001 §3.2, plan/0004 (spans, diagnostics, passes) and
//! schema/ir/v1.json. Change all three sides together
//! (`.agents/skills/gripsack-ir`).

pub mod diagnostic;
pub mod model;
pub mod span;
pub mod step;
pub mod validate;

pub use diagnostic::{codes, Diagnostic, Label, Severity};
pub use model::*;
pub use span::Span;
pub use step::*;
pub use validate::{check, parse, validate, IR_VERSION};
