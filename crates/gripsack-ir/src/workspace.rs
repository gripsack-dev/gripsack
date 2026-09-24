//! v5 typed workspace declarations (schema/ir/v5.json, plan/0052):
//! a named-output catalog the core admits structurally. Every node
//! carries a mandatory provenance `span`; tagged unions (`kind`) are
//! closed by the pass-1.5 tagged-field walk (`tagged.rs`), plain structs
//! by `deny_unknown_fields` — no field is ever silently dropped.
//!
//! Admission judges structure only. The shared execution gate
//! (`execution_gate.rs`) labels an unavailable output with E124
//! before CLI or direct executor entrypoints can open a home, lockfile,
//! worker or scheduler. It never authorizes execution or a fallback.
//!
//! One type family per child module, re-exported flat:
//! `catalog` owns the envelope and nine outputs, `command` the
//! shared command value, `file` profile files/calendar, `layout`
//! absolute install prefixes, and `platform` target/ABI/OS floors.

mod catalog;
mod command;
pub(crate) mod execution_gate;
mod file;
mod layout;
mod platform;

pub use catalog::*;
pub use command::*;
pub use file::*;
pub use layout::*;
pub use platform::*;
