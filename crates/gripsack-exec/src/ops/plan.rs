//! Codegen (0034): observations + the desired state in, Ops out.
//! Every decision a destination needs is computed HERE — apply,
//! rollback, and plan share this module.

use super::*;
use crate::ctx::ExecError;
use store::journal::{Intended, ObjectIdentity};

mod link;
mod merge;
mod remove;
mod restore;
mod write;

use link::plan_link;
use merge::plan_merge;
pub(crate) use remove::plan_remove_op;
pub(crate) use restore::plan_restore_op;
pub use write::WritePermissions;
use write::{plan_migration_write, plan_write};

/// Everything a planner needs to know about one destination — the
/// ONE observation, the lineage record, the run's take-over scope.
/// (A struct, not nine positional arguments: each field is named at
/// the call site.)
pub struct DestView<'a> {
    pub output: &'a str,
    pub entry: &'a DeploymentEntry,
    /// The canonical physical destination (0030 §P0-1).
    pub dest: PathBuf,
    /// The home the store lives under (link "ours" is target-under-home).
    pub home: &'a Path,
    pub observed: Option<crate::deploy::Observation>,
    pub prev: Option<&'a store::DeployedEntry>,
    pub take_over: bool,
}

impl DestView<'_> {
    /// The journal-domain identity of the observation.
    pub(crate) fn observed_identity(&self) -> Option<ObjectIdentity> {
        self.observed.as_ref().map(|o| match o {
            crate::deploy::Observation::Symlink { target } => {
                ObjectIdentity::Link(target.to_string_lossy().into_owned())
            }
            crate::deploy::Observation::File { bytes, mode } => {
                ObjectIdentity::File(store::canonical_bytes_identity(bytes, *mode))
            }
        })
    }

    fn base(&self, kind: OpKind, authority: Option<Authority>, intended: Intended) -> Op {
        Op::new(
            self.output.to_string(),
            self.dest.clone(),
            self.entry.to.clone(),
            self.entry.mode.clone(),
            kind,
            authority,
            self.observed_identity(),
            intended,
            None,
            None,
        )
    }

    fn produces(
        &self,
        hash: store::hash::ManifestHash,
        file_mode: Option<u32>,
        preserved: bool,
    ) -> Option<ProducedEntry> {
        Some(ProducedEntry {
            from: std::path::PathBuf::from(&self.entry.from),
            mode: self.entry.mode.clone(),
            vars: self.entry.vars.clone(),
            hash,
            file_mode,
            source_executable: None,
            prior: None,
            preserved_drift: preserved,
        })
    }
}

/// What the mode's decision needs beyond the view.
pub enum ModeInput<'a> {
    /// Owned link: the store payload it points at, its content hash,
    /// and whether the live link already points there.
    Link {
        source: &'a Path,
        content_hash: store::hash::ManifestHash,
        already: bool,
    },
    /// Tracked copy / template: content plus source policy or exact restoration.
    Write {
        content: &'a [u8],
        permissions: WritePermissions,
    },
    /// An explicitly admitted predecessor link becoming a physical file.
    MigrationWrite {
        authorization: &'a crate::deploy::MigrationEntry,
        content: &'a [u8],
        permissions: WritePermissions,
    },
    /// Merge payload and permissions; hosting text is the view's one observation.
    Merge {
        block_id: Option<&'a store::ManagedBlockId>,
        payload: &'a str,
        permissions: WritePermissions,
    },
}

/// The per-destination decision (0034): one Op out, per the mode's
/// rule — plan_copy for copies/templates, plan_link for owned links,
/// the block hash for merge. THE function plan/apply/rollback share.
pub(crate) fn plan_entry_op(view: &DestView, input: ModeInput) -> Result<Op, ExecError> {
    match input {
        ModeInput::Link {
            source,
            content_hash,
            already,
        } => Ok(plan_link(view, source, content_hash, already)),
        ModeInput::Write {
            content,
            permissions,
        } => Ok(plan_write(view, content, permissions)),
        ModeInput::MigrationWrite {
            authorization,
            content,
            permissions,
        } => plan_migration_write(view, authorization, content, permissions),
        ModeInput::Merge {
            block_id,
            payload,
            permissions,
        } => plan_merge(view, payload, permissions, block_id),
    }
}
