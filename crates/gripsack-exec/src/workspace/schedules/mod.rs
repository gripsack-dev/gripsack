//! Captured finite jobs. Profiles own registration through the existing
//! generation transaction; callbacks acquire that same lifecycle fence before
//! transferring retention to the common task runner.
pub(super) mod capture;
mod dispatch;
pub(crate) mod lifecycle;
mod runner;
mod snapshot;
pub mod systemd;

pub use dispatch::dispatch;
pub use gripsack_store::jobs::{JobId, JobRevision};
pub use lifecycle::{disable, inspect};
pub(crate) use snapshot::inspect_capture;
