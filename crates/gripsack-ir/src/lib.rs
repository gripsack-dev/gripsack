//! The IR contract between frontends and the core: types, spans,
//! diagnostics, and the compiler passes every IR document goes through
//! (plan/0001 §3.2, 0004, 0007). The schema lives in
//! `schema/ir/v1.json`; change all three sides together
//! (`.agents/skills/gripsack-ir`).
//!
//! ```text
//! frontend (python | typescript)
//!     │  evals your modules, emits JSON with spans
//!     ▼
//! parse        E000 malformed · E100 wrong ir_version
//!     ▼
//! sema::run    ordered passes, one concern each:
//!     steps        E103 both-shapes · E106 dup/reserved ids · E104 refs
//!     deps         E101 unknown module
//!     destinations E102 bad destination
//!     resources    E107 undeclared resource
//!     ▼
//! typed Ir  +  span-labeled Diagnostics (stable codes, source snippets)
//! ```
//!
//! To add a check: one file in `sema/`, one line in `PASSES`, one test.

pub mod diagnostic;
pub mod model;
pub mod parse;
pub mod sema;
pub mod span;
pub mod step;
mod tagged;

pub use diagnostic::{Diagnostic, Label, Severity, codes};
pub use model::*;
pub use parse::{IR_VERSION, parse};
/// Backwards-compatible alias: pass 2 is `sema::run`.
pub use sema::run as validate;
pub use sema::{check, run};
pub use span::Span;
pub use step::*;
