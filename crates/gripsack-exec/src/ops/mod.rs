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

/// Removal authority for a Remove op (0046): the manifest entry being
/// removed (its prior is the restore target) and its store path (the
/// exact-link guard, 0030 §15). Carried BY the variant — a Remove
/// without it is unconstructible, so the executor never `expect`s.
#[derive(Debug, Clone)]
pub struct RemoveTarget {
    pub entry: store::DeployedEntry,
    pub store_path: PathBuf,
}

/// What an operation does to its destination.
#[derive(Debug, Clone)]
pub enum OpKind {
    /// Owned link: point the destination at the store payload.
    Link { target: PathBuf },
    /// Tracked copy / template: write content at exactly this mode.
    Write { content: ContentSource, mode: u32 },
    /// Merge upsert, re-derived from the guarded live content at mutation.
    /// Inspection/report notes are computed once by the planner.
    MergeUpsert {
        payload: Vec<u8>,
        marker: Option<String>,
        mode: u32,
    },
    /// Remove the destination, or restore its prior (the removal
    /// target's lineage decides; merge splices only our block out).
    Remove(RemoveTarget),
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

/// One planned operation (0034, hardened 0046): fields are private and
/// the planner's one constructor enforces the coherence table, so every
/// executable op carries authority, observation, intent and payload
/// consistently — a Remove without its removal authority, or a marker
/// op carrying execution data, is unrepresentable outside this module's
/// planner code.
#[derive(Debug, Clone)]
pub struct Op {
    module: String,
    /// The canonical physical destination (0030 §P0-1).
    dest: PathBuf,
    /// The declared spelling (provenance for reports).
    declared_to: String,
    /// The ownership mode (provenance: reports speak in the mode's
    /// voice — a satisfied merge block is not an unchanged file).
    mode: Ownership,
    kind: OpKind,
    /// None on inert ops (Satisfied/Preserved) and markers.
    authority: Option<Authority>,
    /// The ONE observation at plan time (0030 §P0-2) — the journal
    /// precondition re-validates it at the mutation.
    observed: Option<ObjectIdentity>,
    /// The intended end state, journal domain (inert ops: the
    /// observation restated).
    intended: Intended,
    /// The manifest entry this deploy produces (None on inert ops:
    /// Satisfied keeps the previous entry, Preserved re-records the
    /// observation).
    produces: Option<ProducedEntry>,
    /// The preview note for marker ops (Deferred/RunEffect) — why the
    /// decision can't be computed offline.
    note: Option<String>,
}

impl Op {
    /// The coherence table (0046), checked at construction and after
    /// every planner adjustment (debug builds):
    ///
    /// - Link/Write/MergeUpsert carry authority at birth (the manifest
    ///   record may arrive later — fetched identities pin late, 0014;
    ///   its presence is enforced at the execution boundary instead,
    ///   see [`Op::as_executable`]);
    /// - Remove carries Update authority and no manifest record;
    /// - Satisfied carries no authority;
    /// - markers (Deferred/RunEffect) carry a note and nothing else.
    fn check(&self) {
        match &self.kind {
            OpKind::Link { .. } | OpKind::Write { .. } | OpKind::MergeUpsert { .. } => {
                debug_assert!(
                    self.authority.is_some(),
                    "a deploy op carries its authority"
                );
            }
            OpKind::Remove(_) => {
                debug_assert_eq!(self.authority, Some(Authority::Update));
                debug_assert!(self.produces.is_none());
            }
            OpKind::Satisfied => debug_assert!(self.authority.is_none()),
            OpKind::RunEffect | OpKind::Deferred => {
                debug_assert!(self.note.is_some(), "a marker op explains itself");
                debug_assert!(self.authority.is_none() && self.produces.is_none());
            }
            OpKind::Preserved => {}
        }
    }

    /// The planner's one constructor (0034, 0046): see [`Op::check`]
    /// for the coherence table.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        module: String,
        dest: PathBuf,
        declared_to: String,
        mode: Ownership,
        kind: OpKind,
        authority: Option<Authority>,
        observed: Option<ObjectIdentity>,
        intended: Intended,
        produces: Option<ProducedEntry>,
        note: Option<String>,
    ) -> Op {
        let op = Op {
            module,
            dest,
            declared_to,
            mode,
            kind,
            authority,
            observed,
            intended,
            produces,
            note,
        };
        op.check();
        op
    }

    /// The planner fills in the manifest record once it computes it
    /// (fetches pin identities late); coherence re-checked.
    pub(crate) fn with_produces(mut self, produces: Option<ProducedEntry>) -> Op {
        self.produces = produces;
        self.check();
        self
    }

    /// The planner's note attachment (merge reports, marker reasons).
    pub(crate) fn with_note(mut self, note: Option<String>) -> Op {
        self.note = note;
        self.check();
        self
    }

    /// `produces` content access for the planner's late field fills.
    pub(crate) fn produces_mut(&mut self) -> Option<&mut ProducedEntry> {
        self.produces.as_mut()
    }
}

/// An op cleared for execution (0046): marker ops (Deferred/RunEffect)
/// cannot get here — the conversion IS the check, so the executor's
/// match has no `unreachable!` arm.
pub(crate) struct ExecutableOp<'a>(pub(crate) &'a Op);
impl Op {
    /// Preview-only markers fail the conversion with a classified
    /// error (a planner bug, reported, never a panic). Deploy ops must
    /// carry their manifest record by now (0014's late pinning is
    /// done at planning's end — an executed deploy always records).
    pub(crate) fn as_executable(&self) -> Result<ExecutableOp<'_>, crate::ctx::ExecError> {
        match &self.kind {
            OpKind::RunEffect | OpKind::Deferred => Err(crate::ctx::ExecError::Step {
                module: self.module.clone(),
                step: "deploy".into(),
                detail: "a preview-only marker op reached execution — a planner bug".into(),
            }),
            OpKind::Link { .. } | OpKind::Write { .. } | OpKind::MergeUpsert { .. }
                if self.produces.is_none() =>
            {
                Err(crate::ctx::ExecError::Step {
                    module: self.module.clone(),
                    step: "deploy".into(),
                    detail: "a deploy op reached execution without its manifest record \
                             — a planner bug"
                        .into(),
                })
            }
            _ => Ok(ExecutableOp(self)),
        }
    }
}

impl Op {
    // --- read accessors (the fields stay private — 0046) ---

    pub fn module(&self) -> &str {
        &self.module
    }
    /// The canonical physical destination (0030 §P0-1).
    pub fn dest(&self) -> &Path {
        &self.dest
    }
    /// The declared spelling (provenance for reports).
    pub fn declared_to(&self) -> &str {
        &self.declared_to
    }
    pub fn mode(&self) -> &Ownership {
        &self.mode
    }
    pub fn kind(&self) -> &OpKind {
        &self.kind
    }
    pub fn authority(&self) -> Option<Authority> {
        self.authority
    }
    pub fn observed(&self) -> Option<&ObjectIdentity> {
        self.observed.as_ref()
    }
    pub fn intended(&self) -> &Intended {
        &self.intended
    }
    pub fn produces(&self) -> Option<&ProducedEntry> {
        self.produces.as_ref()
    }
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
}

/// What a deploying op records in the manifest.
#[derive(Debug, Clone)]
pub struct ProducedEntry {
    pub from: std::path::PathBuf,
    pub mode: Ownership,
    pub vars: std::collections::BTreeMap<String, String>,
    pub hash: store::hash::ManifestHash,
    pub file_mode: Option<u32>,
    pub source_executable: Option<bool>,
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
pub(crate) use plan::{
    DestView, ModeInput, WritePermissions, plan_entry_op, plan_remove_op, plan_restore_op,
};
pub use preview::preview_ops;
