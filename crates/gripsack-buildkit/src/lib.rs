//! `gripsack-buildkit` — Gripsack's owned BuildKit adapter (Epic B,
//! plan/0051). The Rust/musl core stays the coordinator: this crate
//! owns the bounded bridge protocol, worker lease policy and the
//! independent validation of emitted LLB. It never evaluates
//! TypeScript, resolves packages, owns the store or activates
//! anything (Epic B §5 module boundaries).
//!
//! Current surface: [`protocol`] — the framed Rust↔Go transport with
//! strict decoding and fence epochs (B1-03's wire contract). Worker
//! profiles/provisioning, LLB validation kernels and session
//! lifecycle land with their B1/B2 leaves.

pub mod protocol;
