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
//! (`gripsack-store`) drives it over `RealFs`.
#![forbid(unsafe_code)]

pub mod recover;
pub mod storage;
