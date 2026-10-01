//! `gripsack-conda` — the conda closure half of the frozen workspace build.
//!
//! The core links the bounded protocol and Rattler's canonical record/MatchSpec
//! parser. Resolving, networking and installation remain out of process:
//!
//! * [`protocol`] — the length-prefixed JSON frame protocol (v1) spoken with
//!   the out-of-process `gripsack-conda` helper.
//! * [`client`] — [`client::CondaHelper`], a strict stdio client that spawns
//!   the helper, performs the version handshake, and validates every echo.
//! * [`virtuals`] — frozen virtual-package requirements checked against complete
//!   measured name/version/build facts, not a second partial version grammar.
//!
//! The optional helper links the Rattler solver, network and installer behind
//! the `helper` feature. A response is advisory data: the core independently
//! admits frozen records and verifies the produced tree against pinned archives.

pub mod channels;
pub mod client;
pub mod protocol;
pub mod provision;
pub mod records;
pub mod receipt;
pub mod virtuals;

pub use client::{CondaError, CondaHelper};
pub use channels::{InvalidChannel, canonicalize_channel};
pub use virtuals::{VirtualConstraintError, evaluate_virtual_constraints};
