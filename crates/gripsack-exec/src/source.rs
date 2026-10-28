//! Workspace source resolution, publication, and read-only layout evidence.

mod acquire;
pub(crate) mod preflight;
pub(crate) use acquire::publish;
mod resolve;
pub(crate) use resolve::resolve_spec;
