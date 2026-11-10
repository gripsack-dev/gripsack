//! gripsack ring-0 kernel (plan/0056): pure decisions over typed state.
//!
//! Everything here is plain safe Rust shaped for the Charon/Aeneas
//! translation: no IO, no floats, no `dyn`, no dependencies, `no_std`
//! plus `alloc`. The `proofs` compose gate translates this crate to Lean 4
//! (`proofs/Extracted/`, committed) and proves the kernel theorems over
//! that translation (`proofs/Kernel/`). Proofs are about the shipped code:
//! there is no hand-written Lean copy of these functions.
//!
//! Crate rules: effects stay in the caller's crate; rendering stays out;
//! equality that a theorem depends on is written as an explicit kernel
//! function (`same`) rather than a derived trait, so the proved call graph
//! reaches only the Aeneas standard-library models and no axiomatized
//! external.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod activation;
pub mod apply;
pub mod block;
pub mod convenience;
pub mod gc;
pub mod generation;
pub mod ids;
pub mod journal;
pub mod journal_run;
pub mod ownership;
pub mod publication;
pub mod recover;
pub mod retention;
pub mod rollback;
pub mod selection;

pub use generation::{GenerationId, GenerationList};
pub use ids::{CmdId, DestId, Digest, GenId, TxId};
pub use selection::Classification;
