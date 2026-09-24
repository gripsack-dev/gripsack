//! v4 workspace declarations (schema/ir/v4.json, plan/0052 §2.1–2.2):
//! a named-output catalog the core admits structurally. Every node
//! carries a mandatory provenance `span`; tagged unions (`kind`) are
//! closed by the pass-1.5 tagged-field walk (`tagged.rs`), plain structs
//! by `deny_unknown_fields` — no field is ever silently dropped.
//!
//! Admission judges structure only. Whether a declared capability can
//! execute on this build is a separate, explicit rejection lane owned by
//! the CLI/plan surfaces (E124), never a silent fallback and never
//! decided here.
//!
//! One type family per child module, re-exported flat so
//! `gripsack_ir::workspace::*` keeps its shape: `catalog` (the envelope
//! entry and the nine output types), `command` (the shared command
//! value), `file` (profile file declarations and the calendar grammar).

mod catalog;
mod command;
mod file;

pub use catalog::*;
pub use command::*;
pub use file::*;
