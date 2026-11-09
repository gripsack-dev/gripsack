//! gripsack ring-2 host (plan/0056 §3, §6).
//!
//! For now only the storage layer (F2 step 1): the [`storage::Storage`]
//! trait that the protocol driver will act through, the thin
//! [`storage::RealFs`] over `*at` syscalls, and the crash-injecting
//! [`storage::MemFs`] that mirrors the storage model S1–S3 (plan/0056 §4).
//! Nothing in the product calls this crate yet; F2 step 2 rewires the
//! recover path onto it.
#![forbid(unsafe_code)]

pub mod storage;
