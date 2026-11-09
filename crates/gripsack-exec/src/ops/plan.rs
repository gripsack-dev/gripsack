//! Codegen (0034): observations + the desired state in, Ops out.
//! Every decision a destination needs is computed HERE — apply,
//! rollback, and plan share this module.

use super::*;
use crate::ctx::ExecError;
use gripsack_kernel::ownership::{Consent, Decision, Recorded, Refusal, TakeOver, decide_unit};
use gripsack_kernel::recover::{Ident, Intended as KernelIntended, Live};
use store::journal::{Intended, ObjectIdentity};

mod link;
mod merge;
mod remove;
mod restore;
mod write;

use link::plan_link;
use merge::plan_merge;
pub(crate) use remove::plan_remove_op;
pub(crate) use restore::{Retained, plan_restore_op, retained};
pub use write::WritePermissions;
use write::{plan_migration_write, plan_write};

/// The lineage record of one destination: the previous generation's
/// manifest entry and the store root that generation deployed from. An
/// owned link's recorded identity is the exact link gripsack wrote,
/// `store_path/from`; ownership never follows from where a live link
/// points (plan/0056 K1).
#[derive(Clone, Copy)]
pub struct PreviousRecord<'a> {
    pub entry: &'a store::DeployedEntry,
    pub store_path: &'a Path,
}

impl PreviousRecord<'_> {
    /// What gripsack recorded writing at this destination, as the kernel's
    /// [`Recorded`]. Preserved drift is an observation and confers nothing.
    /// `observed` admits legacy template receipts (a bytes-only hash plus the
    /// landed mode): only an exact match with the live file denotes it.
    pub(crate) fn recorded(&self, observed: Option<&crate::deploy::Observation>) -> Recorded {
        let entry = self.entry;
        if entry.preserved_drift {
            return Recorded::Observed;
        }
        match entry.ownership.policy() {
            Ownership::Owned => Recorded::Wrote(store::hash::kernel::link(
                self.store_path
                    .join(&entry.from)
                    .as_os_str()
                    .as_encoded_bytes(),
            )),
            Ownership::TrackedCopy | Ownership::Template => match observed {
                Some(crate::deploy::Observation::File { bytes, mode })
                    if entry.matches_file(bytes, *mode) =>
                {
                    Recorded::Wrote(store::hash::kernel::file(&store::canonical_bytes_identity(
                        bytes, *mode,
                    )))
                }
                _ => store::hash::kernel::recorded_file(&entry.hash)
                    .map_or(Recorded::Observed, Recorded::Wrote),
            },
            // a managed block is not a whole-object unit (K5, not K1)
            Ownership::Merge => Recorded::Observed,
        }
    }
}

/// An observation as the kernel's [`Live`]: a file is its mode-aware
/// identity, a link the digest of its target bytes.
pub(crate) fn kernel_live(observed: Option<&crate::deploy::Observation>) -> Live {
    match observed {
        None => Live::Absent,
        Some(crate::deploy::Observation::File { bytes, mode }) => Live::Obj(
            store::hash::kernel::file(&store::canonical_bytes_identity(bytes, *mode)),
        ),
        Some(crate::deploy::Observation::Symlink { target }) => {
            Live::Obj(store::hash::kernel::link(target.as_encoded_bytes()))
        }
    }
}

/// The user's take-over answer for one destination as the kernel's
/// [`Consent`] (`--take-over`, an adoption set, or nothing).
pub fn take_over_consent(granted: bool) -> Consent {
    Consent {
        take_over: if granted {
            TakeOver::Granted
        } else {
            TakeOver::Withheld
        },
    }
}

/// Everything a planner needs to know about one destination — the
/// ONE observation, the lineage record, the user's consent.
/// (A struct, not nine positional arguments: each field is named at
/// the call site.)
pub struct DestView<'a> {
    pub output: &'a str,
    pub entry: &'a DeploymentEntry,
    /// The canonical physical destination (0030 §P0-1).
    pub dest: PathBuf,
    pub home: &'a Path,
    pub observed: Option<crate::deploy::Observation>,
    pub prev: Option<PreviousRecord<'a>>,
    pub consent: Consent,
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

    /// The previous generation's entry for this destination.
    pub(crate) fn prev_entry(&self) -> Option<&store::DeployedEntry> {
        self.prev.map(|record| record.entry)
    }

    /// The proved per-unit decision (plan/0056 K1) for deploying `intended`.
    pub(crate) fn decide(&self, intended: Ident) -> Decision {
        let recorded = self.prev.map_or(Recorded::Unrecorded, |record| {
            record.recorded(self.observed.as_ref())
        });
        decide_unit(
            &recorded,
            &kernel_live(self.observed.as_ref()),
            &KernelIntended::Obj(intended),
            &self.consent,
        )
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
    /// Owned link: the store payload it points at and its content hash.
    Link {
        source: &'a Path,
        content_hash: store::hash::ManifestHash,
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
/// rule — the kernel's `decide_unit` for copies, templates and owned
/// links, the block hash for merge. THE function plan/apply/rollback share.
pub(crate) fn plan_entry_op(view: &DestView, input: ModeInput) -> Result<Op, ExecError> {
    match input {
        ModeInput::Link {
            source,
            content_hash,
        } => plan_link(view, source, content_hash),
        ModeInput::Write {
            content,
            permissions,
        } => plan_write(view, content, permissions),
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
