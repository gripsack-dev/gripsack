//! Shared workspace domains: acquisition provenance, lifecycle, destinations,
//! platform identity and operation-specific capability admission.

mod catalog;
pub(crate) mod execution_gate;
mod file;
mod layout;
mod platform;

pub use catalog::*;
pub use execution_gate::WorkspaceOperation;
pub use file::*;
pub use layout::*;
pub use platform::*;
