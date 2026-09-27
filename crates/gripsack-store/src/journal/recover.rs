//! Reconcile: the next run after a crash restores every uncommitted
//! journal entry to its prior state, under the drift guard (0019,
//! 0025 §B). The drift guard: when the entry knows the post-mutation
//! identity and the destination no longer matches it, someone touched
//! the file after the crash — their edit wins, the entry is dropped
//! with a warning. Never delete user edits.

use gripsack_fs::Dir;

/// One recovery outcome from [`reconcile`], typed so callers render
/// severity instead of parsing message text: a kept post-crash edit
/// is a WARNING (your file was left in a state the manifest doesn't
/// know); restores and no-ops are informational.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryNote {
    pub severity: NoteSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteSeverity {
    Info,
    Warn,
}

impl std::fmt::Display for RecoveryNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
use std::io;
use std::path::{Path, PathBuf};

use super::marker::{Classification, RecoveryFacts, classify, cleanup, run_marker};
use super::storage::Journal;
use super::{
    Entry, Intended, ObjectIdentity, PriorSerde, dest_capability, live_identity, prior_identity,
    read_uncommitted,
};

/// What an uncommitted entry asks for, once resolved against the
/// current filesystem.
pub(crate) enum Recovery {
    /// Restore the prior state; `String` describes it for the report.
    Restore(String),
    /// The destination drifted after the crash — the user's now.
    Keep(String),
    /// Live state equals the prior, either never mutated or restored by an
    /// earlier recovery. Durability must still be sealed before cleanup.
    Unchanged,
}

/// Resolve uncommitted journal entries from an interrupted run:
/// committed runs (the flip landed) are cleaned up, their content
/// stands; uncommitted runs are restored to their priors. Returns one
/// human line per decision for the apply report. Must run under the
/// lifecycle lock.
pub fn reconcile(home: &Dir, home_path: &Path) -> io::Result<Vec<RecoveryNote>> {
    let Some(journal) = Journal::open(home)? else {
        return Ok(Vec::new());
    };
    let entries = read_uncommitted(&journal)?;
    // the commit decision by EXACT transaction identity (0026 §4):
    // current == target committed, current == previous uncommitted,
    // anything else is ambiguous and BLOCKS (fail closed — the
    // lifecycle lock serializes runs, so a third value means
    // corruption or tampering, never a branch to guess). A marker
    // missing `previous_generation` fails closed at parse — torn or
    // corrupt, never mistaken for a fresh-machine run.
    let committed = match run_marker(&journal)? {
        Some(marker) => {
            marker.admit_recovery()?;
            // the ONE current-pointer reader (0030 §H10): recovery
            // never uses weaker commit evidence than normal commands
            let current = crate::generations::current_selection_in(home_path, home)?;
            if let Some(selection) = &current {
                // A matching pointer is not authority over a corrupt or
                // missing generation. Admit the same pinned state before
                // either restoration or committed cleanup can have effects.
                crate::generations::read_manifest_at(home, home_path, selection.generation())?;
            }
            match classify(&RecoveryFacts {
                previous: marker.previous.as_ref(),
                target: &marker.target,
                current: current.as_ref(),
            }) {
                Classification::Committed => true,
                Classification::Uncommitted => false,
                Classification::Ambiguous => {
                    return Err(io::Error::other(format!(
                        "journal selections ({:?} → {:?}) match neither current \
                         ({current:?}) nor the recorded predecessor — the \
                         journal is retained; inspect $GRIPSACK_HOME/journal \
                         before running again",
                        marker.previous, marker.target
                    )));
                }
            }
        }
        None => false,
    };
    let mut lines = Vec::new();
    if committed {
        // A process can die after current's rename but before its directory
        // barrier. A later reader sees the new pointer in the kernel cache;
        // seal that observation before durable cleanup destroys the journal.
        gripsack_fs::fsync_dir(home, Path::new("."))?;
        cleanup(
            &journal,
            &entries.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(),
        )?;
        lines.push(RecoveryNote {
            severity: NoteSeverity::Info,
            message: "interrupted run's generation had already activated — journal \
             cleared, deployed state stands"
                .to_string(),
        });
        return Ok(lines);
    }
    let mut entry_paths = Vec::new();
    for (path, entry) in entries {
        let dest = PathBuf::from(&entry.dest);
        // the drift check and the restore share ONE pinned parent
        // inode: a parent symlink swapped between decide() and
        // restore() cannot redirect the recovery write (plan/0021)
        let (dest_dir, dest_name) = dest_capability(&dest)?;
        match decide(&dest_dir, &dest_name, &entry, home)? {
            Recovery::Restore(what) => {
                restore(&dest_dir, &dest_name, &entry.prior, home)?;
                // recovery is held to the transaction's standard
                // (0029 §4): verify the prior identity before the
                // entry may be dropped
                let live = live_identity(&dest_dir, &dest_name)?;
                let expected = prior_identity(&entry.prior, home)?;
                if live != expected {
                    return Err(io::Error::other(format!(
                        "recovery of {} did not produce the prior state — the                          journal is retained; inspect $GRIPSACK_HOME/journal",
                        entry.dest
                    )));
                }
                lines.push(RecoveryNote {
                    severity: NoteSeverity::Info,
                    message: format!("recovered {}: {what}", entry.dest),
                });
            }
            Recovery::Unchanged => {
                // A previous recovery may have made the prior visible and
                // then failed its durability barrier. Observation alone is
                // not authority to discard the still-needed journal entry.
                seal_observed_prior(&dest_dir, &dest_name, &entry.prior)?;
                lines.push(RecoveryNote {
                    severity: NoteSeverity::Info,
                    message: format!("unchanged {}: prior state is durable", entry.dest),
                });
            }
            Recovery::Keep(why) => {
                // a kept post-crash edit is the outcome the user must
                // NOTICE — it means live state and the manifest now
                // disagree about who owns the bytes
                lines.push(RecoveryNote {
                    severity: NoteSeverity::Warn,
                    message: format!("kept {}: {why}", entry.dest),
                });
            }
        }
        entry_paths.push(path);
    }
    cleanup(&journal, &entry_paths)?;
    Ok(lines)
}

/// Intent-based recovery (0026 §6), extended by v2's immediate pre-state
/// for repeated writes. The original prior remains the restore point.
/// Entry admission supplies typed identities; the decision compares those
/// values rather than reparsing strings or inferring an operation direction.
fn decide(dest_dir: &Dir, dest_name: &Path, entry: &Entry, home: &Dir) -> io::Result<Recovery> {
    let live = live_identity(dest_dir, dest_name)?;
    let prior_id = prior_identity(&entry.prior, home)?;
    let before_id = if entry.before.as_ref() == Some(&entry.prior) {
        None
    } else {
        entry
            .before
            .as_ref()
            .map(|before| prior_identity(before, home))
            .transpose()?
            .flatten()
    };
    let intended = Intended::from_wire(&entry.after);
    Ok(
        match decide_from(
            live.as_ref(),
            &intended,
            prior_id.as_ref(),
            before_id.as_ref(),
        ) {
            RecoveryDecision::Restore => Recovery::Restore(describe_prior(&entry.prior)),
            RecoveryDecision::Keep => {
                Recovery::Keep("changed since the interrupted run — your edit stands".into())
            }
            RecoveryDecision::Unchanged => Recovery::Unchanged,
        },
    )
}

/// The report text for a restore — presentation, kept OUT of the
/// decision kernel (0045 F1: the pure outcome never carries its
/// formatting).
fn describe_prior(prior: &PriorSerde) -> String {
    match prior {
        PriorSerde::Absent => "removed (was absent before the interrupted run)".into(),
        PriorSerde::File { .. } => "prior bytes restored".into(),
        PriorSerde::Symlink { target } => format!("prior symlink → {target} restored"),
    }
}

/// The recovery outcome as a pure decision (0045 F1) — the model
/// checkers drive this, so it carries no rendered text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryDecision {
    /// Restore the prior state.
    Restore,
    /// The destination drifted after the crash — the user's now.
    Keep,
    /// Live state equals the prior; the caller still seals it before cleanup.
    Unchanged,
}

/// The recovery decision as a pure function of the three typed
/// identities (0028, 0045 F1): the model checker drives THIS code
/// with abstract states — the protocol's decision logic is what gets
/// checked, not a parallel reimplementation. Typed equality makes the
/// old wire-collision class (a link target spelling a file identity
/// or the removal sentinel) unrepresentable.
pub(crate) fn decide_from(
    live: Option<&ObjectIdentity>,
    intended: &Intended,
    prior_id: Option<&ObjectIdentity>,
    before_id: Option<&ObjectIdentity>,
) -> RecoveryDecision {
    match live {
        // the mutation landed intact
        Some(l) if intended.satisfied_by(l) => RecoveryDecision::Restore,
        // The prior is visible: the original mutation or an earlier restore
        // may explain it. Visibility alone does not establish durability.
        Some(l) if Some(l) == prior_id => RecoveryDecision::Unchanged,
        // A later mutation's durable record may exist before its write.
        // Its admitted predecessor is still ours, never a foreign edit.
        Some(l) if Some(l) == before_id => RecoveryDecision::Restore,
        Some(_) => RecoveryDecision::Keep,
        // absent now: a landed removal or a never-landed creation —
        // either way the prior comes back
        None if prior_id.is_some() => RecoveryDecision::Restore,
        None => RecoveryDecision::Unchanged,
    }
}

fn restore(dest_dir: &Dir, dest_name: &Path, prior: &PriorSerde, home: &Dir) -> io::Result<()> {
    match prior {
        PriorSerde::Absent => {
            // remove_file never removes a directory; a dest that grew
            // into one errors here (ENOTDIR) and the journal entry is
            // retained — recovery data is never dropped on a failed
            // removal (0029 §4)
            match gripsack_fs::remove_file(dest_dir, dest_name) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            gripsack_fs::fsync_dir(dest_dir, Path::new("."))?;
        }
        PriorSerde::File { hash, mode } => {
            let bytes = crate::prior::read_blob(home, hash)?;
            // the mode rides the write (0027 §6): temp → exact mode →
            // fsync → rename, so a restored 0600 secret never exists
            // at a wider mode, not even for the rename's instant
            gripsack_fs::atomic_write_with_mode(dest_dir, dest_name, &bytes, mode.bits())?
        }
        PriorSerde::Symlink { target } => {
            gripsack_fs::symlink_replace(dest_dir, dest_name, Path::new(target))?;
        }
    }
    Ok(())
}

/// Seal an observed prior before cleanup, including a partially completed
/// restoration from an earlier process. Regular bytes need their own barrier;
/// absence/link identity needs the pinned parent-directory barrier.
fn seal_observed_prior(dest_dir: &Dir, dest_name: &Path, prior: &PriorSerde) -> io::Result<()> {
    if matches!(prior, PriorSerde::File { .. }) {
        let file = gripsack_fs::open_file_nofollow(dest_dir, dest_name)?;
        gripsack_fs::fault::operation(gripsack_fs::fault::Boundary::FileSync, dest_name, || {
            file.sync_all()
        })?;
    }
    gripsack_fs::fsync_dir(dest_dir, Path::new("."))
}
