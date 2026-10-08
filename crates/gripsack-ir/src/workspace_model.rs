//! Catalog execution model introduced in v6 and shared by strict v6/v7 readers.
//! v7 adds reviewed Conda floors and package-scoped host runtime authority;
//! version dispatch keeps those fields forbidden in retained v6 envelopes.
mod catalog;
mod command;
mod file;
pub mod graph;
mod host_runtime;
pub mod identity;
mod image;
pub mod lock;
mod source;

pub use crate::workspace::{
    HostAccess, InstallPrefix, LinuxWorker, PackageLayout, RecipeOutputKind, WorkspaceCalendar,
    WorkspaceDestination, WorkspacePlatform,
};
pub use catalog::*;
pub use command::*;
pub use file::*;
pub use host_runtime::{HostLibraryDirectory, HostRuntimeRequirements};
pub use image::*;
pub use source::{
    AcquisitionSource, CondaEnvironmentSource, CondaLibcRequirement, CondaSystemRequirements,
    LockedCondaSource, LockedPixiSource, LockedSource, PixiLockSource,
};
