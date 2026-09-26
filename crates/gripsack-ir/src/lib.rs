//! The IR contract between frontends and the core: types, spans,
//! diagnostics, and the compiler passes every IR document goes through
//! (plan/0001 §3.2, 0004, 0007). The schemas live in `schema/ir/`
//! (v3 legacy modules, strict read-only v4 workspace, current v5
//! typed workspace — plan/0052); change all sides together
//! (`.agents/skills/gripsack-ir`).
//!
//! ```text
//! frontend (typescript)
//!     │  evals your modules, emits JSON with spans
//!     ▼
//! parse        E000 malformed · E100 wrong ir_version (accepts 3..=5)
//!     ▼
//! sema::run    ordered passes, one concern each:
//!     steps        E103 both-shapes · E106 dup/reserved ids · E104 refs
//!     deps         E101 unknown module
//!     destinations E102 bad destination
//!     resources    E107 undeclared resource
//!     workspace    E125 dup outputs · E126 refs · E127 cycles · E128 context
//!     ▼
//! typed Ir  +  span-labeled Diagnostics (stable codes, source snippets)
//! ```
//!
//! To add a check: one file in `sema/`, one line in `PASSES`, one test.

pub mod dependencies;
pub mod diagnostic;
pub mod host_name;
pub mod legacy_v4;
pub mod model;
pub mod parse;
pub mod placeholders;
pub mod prepared;
pub mod sema;
pub mod span;
pub mod step;
mod tagged;
pub mod workspace;

pub use diagnostic::{Diagnostic, Label, Severity, codes};
pub use host_name::HostName;
pub use model::*;
pub use parse::{ACCEPTED_IR_VERSIONS, IR_VERSION, LEGACY_IR_VERSION, WORKSPACE_V4_VERSION, parse};
/// Backwards-compatible alias: pass 2 is `sema::run`.
pub use sema::run as validate;
pub use sema::{check, run};
pub use span::Span;
pub use step::*;
pub use workspace::*;
