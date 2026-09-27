//! gripsack decision kernels (plan/0046): the small, pure policy
//! functions whose correctness the whole transaction protocol leans
//! on, in one dependency-light crate so ONE implementation serves
//! production, the Rust explorers, and the verifier.
//!
//! Verification: `cargo verus verify -p gripsack-policy` (the compose
//! `verify` gate, scripts/check_verus.sh) proves the contracts. Plain
//! `cargo build` compiles the same source with specifications erased,
//! so the musl release and every ordinary build need no verifier.
//!
//! Crate rules (handoff §6.1): no filesystem, network, subprocess,
//! tracing, or async dependencies. Effects stay in the caller's crate;
//! rendering stays out of the kernels.
use vstd::prelude::*;

pub mod activation;
pub mod generation;
pub use generation::{GenerationId, GenerationInventory, GenerationList};
pub mod graph;
pub mod merge;
pub mod ownership;
pub mod retention;
pub mod schedule;
pub mod selection;
pub mod target;

verus! {

/// The commit classification of an interrupted run (0019, 0026 §4,
/// 0028): recovery compares the run marker's declared transaction
/// against the live `current` pointer by EXACT identity. Numeric
/// ordering never decides — a crashed roll-forward has
/// current < target before the flip and is not committed by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// The flip landed: the generation owns the truth; journal
    /// cleanup only.
    Committed,
    /// The flip never landed: restore every journaled prior.
    Uncommitted,
    /// Neither: corruption or tampering — recovery changes nothing
    /// and keeps the journal intact (fail closed).
    Ambiguous,
}

}
