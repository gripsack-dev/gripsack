//! Offline, data-only semantic review. Admission owns resource/privacy bounds;
//! capture projects existing IR/graph/receipt APIs; comparison never plans or executes.
pub mod admission;
mod capture;
mod compare;
pub mod config_view;
pub(crate) mod evidence;
mod identities;
mod jobs;
mod model;
mod normalize;
pub mod operations;
pub(crate) mod retention;
mod selection;

pub use admission::{parse_snapshot, read_snapshot, write_snapshot_private};
pub(crate) use capture::capture_profile;
pub use capture::{
    capture_current, capture_generation, capture_ir, capture_ir_at, capture_proposed,
};
pub use capture::{capture_evaluated, capture_repository};
pub use compare::{
    Change, ChangeSet, Difference, GraphSide, Impact, ImpactKind, ReasonEdge, compare,
};
pub use model::*;

#[cfg(test)]
mod tests;
