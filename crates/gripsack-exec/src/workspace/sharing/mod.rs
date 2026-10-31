//! Passive native artifact sharing. Format admission, cryptographic trust,
//! transport and local publication remain separate authority boundaries.
//! Managed consumers re-admit locally configured trust; transport never exposes
//! commands or registers automation. The store owns durability and retention.
mod admission;
mod closure;
mod conda;
mod environment;
mod environment_projection;
pub mod format;
mod pack;
mod receipt;
pub mod registry;
mod runtime;
pub mod target;
pub mod trust;

pub(in crate::workspace) use admission::{ReadmissionScope, Snapshot as RetainedInputSnapshot};
pub use closure::{ClosureBundle, ExternalManifest};
pub use conda::PortableConda;
pub(in crate::workspace) use environment_projection::{
    acquire, admit_projected_command, inspect_source, projected_entries, readmit_projection,
};
pub use pack::pack;
pub use runtime::{
    ExternalPackageFacts, PackageFacts, export_bundle, import_bundle, inspect, readmit_roots,
};

#[cfg(test)]
mod format_tests;
