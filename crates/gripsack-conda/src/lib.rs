//! `gripsack-conda` — the conda closure half of the frozen workspace build.
//!
//! The core links the bounded protocol and Rattler's canonical record/MatchSpec
//! parser. Resolving, networking and installation remain out of process:
//!
//! * [`protocol`] — the one-shot length-prefixed protocol (v2) spoken with
//!   the out-of-process `gripsack-conda` helper.
//! * [`client`] — [`client::CondaHelper`], exact executable selection and
//!   bounded native transactions with version and response-identity admission.
//! * [`virtuals`] — frozen virtual-package requirements checked against complete
//!   measured name/version/build facts, not a second partial version grammar.
//!
//! The separately built `tools/conda-helper` workspace links the Rattler solver,
//! network and installer. Its private dependency lock does not constrain or
//! prevent publication of the core library. Helper responses remain advisory:
//! the core admits frozen records and verifies trees against pinned archives.

pub mod channels;
pub mod client;
pub mod protocol;
pub mod provision;
pub mod receipt;
pub mod records;
pub mod virtuals;

pub use channels::{InvalidChannel, canonicalize_channel};
pub use client::{CondaError, CondaHelper};
pub use virtuals::{VirtualConstraintError, evaluate_requirements};
