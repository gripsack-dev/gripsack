//! v5 typed workspace declarations (schema/ir/v5.json, plan/0052):
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
//! One type family per child module, re-exported flat:
//! `catalog` owns the envelope and nine outputs, `command` the
//! shared command value, `file` profile files/calendar, `layout`
//! absolute install prefixes, and `platform` target/ABI/OS floors.

mod catalog;
mod command;
mod file;
mod layout;
mod platform;

pub use catalog::*;
pub use command::*;
pub use file::*;
pub use layout::*;
pub use platform::*;
