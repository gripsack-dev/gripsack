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
//! Ring-0 decisions proved in Lean live in `gripsack-kernel` (plan/0056);
//! what remains here is still Verus-checked.
//!
//! Crate rules (handoff §6.1): no filesystem, network, subprocess,
//! tracing, or async dependencies. Effects stay in the caller's crate;
//! rendering stays out of the kernels.
pub mod conda_admission;
pub mod graph;
pub mod operation_budget;
pub mod process_budget;
pub mod rate_limit;
pub mod retry_budget;
pub mod semantic;
pub mod target;
pub mod task;
pub mod update_survey;
pub mod workspace_command;
