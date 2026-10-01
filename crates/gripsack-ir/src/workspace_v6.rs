//! Current workspace wire: explicit pinned Linux execution, shared command
//! descriptions and pure input/lock references. Historical v5 types remain a
//! separate reader; missing old fields never acquire new execution authority.
mod catalog;
mod command;
mod file;
mod image;
mod source;
pub mod graph;
pub mod identity;
pub mod lock;

pub use crate::workspace::{
    HostAccess, InstallPrefix, LinuxWorker, PackageLayout, RecipeOutputKind, WorkspaceCalendar,
    WorkspaceDestination, WorkspacePlatform,
};
pub use catalog::*;
pub use command::*;
pub use file::*;
pub use image::*;
pub use source::{
    CondaEnvironmentSource, LockedCondaSource, LockedPixiSource, LockedSource, PixiLockSource,
    WorkspaceSourceV6,
};
