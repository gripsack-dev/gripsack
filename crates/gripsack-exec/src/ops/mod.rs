//! The one operation list (plan/0034): one planner, three consumers.
//! `plan` renders the list, `apply` executes it, rollback plans with
//! a target manifest as the desired state. The reviewer's shape:
//! every operation carries its destination, provenance, observed
//! identity, intended end state, authority, and recovery behavior.
//!
//! The honest contract is unchanged: plan is a preview from observed
//! state; the journal precondition re-validates every observation at
//! the mutation. One planner makes the DECISIONS identical — the
//! world is allowed to move.

use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use std::path::{Path, PathBuf};
use store::journal::{Intended, ObjectIdentity};

/// Where an operation's content comes from. `DeferredFetch` is the
/// plan-time form for fetched payloads: the op exists and renders,
/// the identity is pinned at apply (0014's contract).
#[derive(Debug, Clone)]
pub enum ContentSource {
    /// Bytes known now (repo file, rendered template, recorded store
    /// payload on rollback).
    Bytes(Vec<u8>),
    /// The payload is fetched/staged at apply — decision deferred.
    DeferredFetch,
}

/// The lineage answer for this destination (0029, 0030 §H5): what
/// authority the write carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authority {
    /// Nothing there (or only our own deployed object).
    Fresh,
    /// An authorized update of what we deployed.
    Update,
    /// An explicit absorb (`--take-over`): the prior is captured,
    /// the epoch's FIRST origin is retained.
    TakeOver,
    /// The destination is foreign and unrecorded — apply refuses,
    /// rollback keeps.
    Foreign,
}

/// What an operation does to its destination.
#[derive(Debug, Clone)]
pub enum OpKind {
    /// Owned link: point the destination at the store payload.
    Link { target: PathBuf },
    /// Tracked copy / template: write content at exactly this mode.
    Write { content: ContentSource, mode: u32 },
    /// Merge: upsert our block in the foreign file. `existing` is the
    /// plan-time file text — the report's notes derive from it; the
    /// splice itself re-derives from the latest content at the
    /// mutation (0029 §3).
    MergeUpsert {
        payload: Vec<u8>,
        marker: Option<String>,
        existing: String,
    },
    /// Remove the destination, or restore its prior (the entry's
    /// lineage decides; merge splices only our block out).
    Remove,
    /// The live object already matches the intent.
    Satisfied,
    /// A run/shell step — an opaque effect (0033 R5): the preview
    /// marks it, apply executes it (outside the op list's journal —
    /// run steps are declared non-transactional, 0007).
    RunEffect,
    /// The payload is fetched at apply (0014): the op renders, the
    /// decision pins then. Plan-time only.
    Deferred,
    /// User drift is preserved — recorded, never overwritten.
    Preserved,
}

/// One planned operation.
#[derive(Debug, Clone)]
pub struct Op {
    pub module: String,
    /// The canonical physical destination (0030 §P0-1).
    pub dest: PathBuf,
    /// The declared spelling (provenance for reports).
    pub declared_to: String,
    /// The ownership mode (provenance: reports speak in the mode's
    /// voice — a satisfied merge block is not an unchanged file).
    pub mode: Ownership,
    pub kind: OpKind,
    /// None on inert ops (Satisfied/Preserved).
    pub authority: Option<Authority>,
    /// The ONE observation at plan time (0030 §P0-2) — the journal
    /// precondition re-validates it at the mutation.
    pub observed: Option<ObjectIdentity>,
    /// The intended end state, journal domain (inert ops: the
    /// observation restated).
    pub intended: Intended,
    /// The manifest entry this deploy produces (None on inert ops:
    /// Satisfied keeps the previous entry, Preserved re-records the
    /// observation).
    pub produces: Option<ProducedEntry>,
    /// The preview note for marker ops (Deferred/RunEffect) — why the
    /// decision can't be computed offline.
    pub note: Option<String>,
    /// Removal authority for Remove ops: the manifest entry being
    /// removed (its prior is the restore target) and its store path
    /// (the exact-link guard, 0030 §15).
    pub removing: Option<(store::DeployedEntry, PathBuf)>,
}

/// What a deploying op records in the manifest.
#[derive(Debug, Clone)]
pub struct ProducedEntry {
    pub from: std::path::PathBuf,
    pub mode: Ownership,
    pub vars: std::collections::BTreeMap<String, String>,
    pub hash: store::hash::ManifestHash,
    pub file_mode: Option<u32>,
    pub prior: Option<store::Prior>,
    pub preserved_drift: bool,
}

/// The report row an executed (or previewed) op produces.
pub struct OpReport {
    pub summary: String,
    pub kind: ReportKind,
}

pub(crate) mod execute;
pub mod plan;
pub mod preview;

#[cfg(test)]
mod model;

pub(crate) use execute::execute_op;
pub(crate) use plan::{DestView, ModeInput, plan_entry_op, plan_remove_op, plan_restore_op};
pub use preview::preview_ops;
