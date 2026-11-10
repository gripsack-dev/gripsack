//! gripsack policy functions: small, pure admission rules and resource
//! budgets shared by the IR, the process supervisor and the fetchers.
//!
//! Plain Rust checked by unit tests. Decisions that carry a safety claim
//! users rely on live in `gripsack-kernel`, where they are proved in Lean
//! (plan/0056); what remains here is either a finite admission table (the
//! tests enumerate every input) or resource-budget arithmetic (§8 of plan
//! 0056: rate limits, retries, deadlines and process I/O stay out of the
//! kernel).
//!
//! Crate rules: no filesystem, network, subprocess, tracing, or async
//! dependencies. Effects stay in the caller's crate; rendering stays out.
pub mod conda_admission;
pub mod graph;
pub mod operation_budget;
pub mod process_budget;
pub mod rate_limit;
pub mod retry_budget;
pub mod target;
pub mod task;
pub mod workspace_command;
