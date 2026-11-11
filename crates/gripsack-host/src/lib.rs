//! gripsack ring-2 host (plan/0056 §3, §6).
//!
//! The storage layer (F2 step 1): the [`storage::Storage`] trait the
//! protocol drivers act through, the thin [`storage::RealFs`] over `*at`
//! syscalls, and the crash-injecting [`storage::MemFs`] that mirrors the
//! storage model S1–S3 and injects S5 external writes (plan/0056 §4).
//!
//! The recover driver (F2 step 2): [`recover::recover`] runs the kernel's
//! recover machine over any `Storage`, re-checking each act against the
//! identity the kernel decided on. The product's journal recovery
//! ([`store::journal`]) drives it over `RealFs`.
//!
//! The journal-run driver (F2 step 3): [`journal_run::RunDriver`] runs the
//! kernel's journal-run machine — the marker, the leaf effect protocol per
//! destination, the commit or end and the cleanup — over any `Storage`. The
//! product's journal ([`store::journal`]) drives it over `RealFs`.
//!
//! Absorbed in F2 step 10b (plan/0056 §24):
//!
//! - [`fs`] — the cap-std filesystem primitives (formerly `gripsack-fs`);
//! - [`process`] — the bounded process supervisor and its I/O and
//!   operation budgets (formerly `gripsack-process` and the budgets of
//!   `gripsack-policy`);
//! - [`store`] — the store layout and wire codecs: paths, content hashes,
//!   the journal, manifests, selections, generations, activation storage,
//!   schedule identities (formerly `gripsack-store`).
//!
//! The protocol drivers and the storage layer forbid `unsafe`; the absorbed
//! modules keep the narrow `libc` calls they always had.

pub mod fs;
pub mod journal_run;
pub mod process;
pub mod recover;
pub mod storage;
pub mod store;
