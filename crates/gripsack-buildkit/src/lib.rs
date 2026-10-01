//! `gripsack-buildkit` — Gripsack's owned BuildKit adapter (Epic B,
//! plan/0051). The Rust/musl core stays the coordinator: this crate
//! owns the bounded bridge protocol, worker lease policy and the
//! independent validation of emitted LLB. It never evaluates
//! TypeScript, resolves packages, owns the store or activates
//! anything (Epic B §5 module boundaries).
//!
//! [`plan`] admits the production projection, [`llb`] independently checks
//! actual upstream definition bytes, and [`transport`] submits only that
//! checked result through the shared bounded native-process supervisor.
//! Worker ownership remains separate from host artifact publication.

pub mod identity;
pub mod llb;
pub mod oci;
pub mod plan;
pub mod protocol;
pub mod transport;
pub mod worker;
