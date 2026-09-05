//! Reconcile: the next run after a crash restores every uncommitted
//! journal entry to its prior state, under the drift guard (0019,
//! 0025 §B). The drift guard: when the entry knows the post-mutation
//! identity and the destination no longer matches it, someone touched
//! the file after the crash — their edit wins, the entry is dropped
//! with a warning. Never delete user edits.

use gripsack_fs::Dir;
use std::io;
use std::path::{Path, PathBuf};

use super::marker::{Classification, RecoveryFacts, classify, cleanup, run_marker};
use super::{
    Entry, PriorSerde, dest_capability, live_identity, prior_blob_rel, prior_identity,
    read_uncommitted,
};

/// What an uncommitted entry asks for, once resolved against the
/// current filesystem.
pub(crate) enum Recovery {
    /// Restore the prior state; `String` describes it for the report.
    Restore(String),
    /// The destination drifted after the crash — the user's now.
    Keep(String),
    /// Live state IS the prior: the mutation never landed (crash
    /// between record and write) — nothing to restore.
    Unchanged,
}

/// Resolve uncommitted journal entries from an interrupted run:
/// committed runs (the flip landed) are cleaned up, their content
/// stands; uncommitted runs are restored to their priors. Returns one
/// human line per decision for the apply report. Must run under the
/// lifecycle lock.
pub fn reconcile(home: &Dir, home_path: &Path) -> io::Result<Vec<String>> {
    let Some(entries) = read_uncommitted(home)? else {
        return Ok(Vec::new());
    };
    // the commit decision by EXACT transaction identity (0026 §4):
    // current == target committed, current == previous uncommitted,
    // anything else is ambiguous and BLOCKS (fail closed — the
    // lifecycle lock serializes runs, so a third value means
    // corruption or tampering, never a branch to guess). A marker
    // missing `previous_generation` fails closed at parse — torn or
    // corrupt, never mistaken for a fresh-machine run.
    let committed = match run_marker(home)? {
        Some(marker) => {
            // the ONE current-pointer reader (0030 §H10): recovery
            // never uses weaker commit evidence than normal commands
            let current = crate::generations::current(home_path)?;
            match classify(&RecoveryFacts {
                previous: marker.previous_generation,
                target: marker.target_generation,
                current,
            }) {
                Classification::Committed => true,
                Classification::Uncommitted => false,
                Classification::Ambiguous => {
                    return Err(io::Error::other(format!(
                        "journal run marker ({:?}→{}) matches neither current \
                         ({current:?}) nor its previous generation — the \
                         journal is retained; inspect $GRIPSACK_HOME/journal \
                         before running again",
                        marker.previous_generation, marker.target_generation
                    )));
                }
            }
        }
        None => false,
    };
    let mut lines = Vec::new();
    if committed {
        cleanup(
            home,
            &entries.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(),
        )?;
        lines.push(
            "interrupted run's generation had already activated — journal \
             cleared, deployed state stands"
                .to_string(),
        );
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
                if live.as_deref() != expected.as_deref() {
                    return Err(io::Error::other(format!(
                        "recovery of {} did not produce the prior state — the                          journal is retained; inspect $GRIPSACK_HOME/journal",
                        entry.dest
                    )));
                }
                lines.push(format!("recovered {}: {what}", entry.dest));
            }
            Recovery::Unchanged => {
                lines.push(format!(
                    "unchanged {}: the mutation never landed",
                    entry.dest
                ));
            }
            Recovery::Keep(why) => {
                lines.push(format!("kept {}: {why}", entry.dest));
            }
        }
        entry_paths.push(path);
    }
    cleanup(home, &entry_paths)?;
    Ok(lines)
}

/// Three-way, intent-based (0026 §6): the entry recorded the intended
/// post-state BEFORE the mutation, so a post-crash edit is
/// distinguishable from the mutation itself.
fn decide(dest_dir: &Dir, dest_name: &Path, entry: &Entry, home: &Dir) -> io::Result<Recovery> {
    let live = live_identity(dest_dir, dest_name)?;
    let prior_id = prior_identity(&entry.prior, home)?;
    Ok(decide_from(
        live.as_deref(),
        &entry.after,
        prior_id.as_deref(),
        &entry.prior,
    ))
}

/// The recovery decision as a pure function of the three identities
/// (0028): the model checker drives THIS code with abstract states —
/// the protocol's decision logic is what gets checked, not a parallel
/// reimplementation.
pub(crate) fn decide_from(
    live: Option<&str>,
    intended: &str,
    prior_id: Option<&str>,
    prior: &PriorSerde,
) -> Recovery {
    let restore_what = || match prior {
        PriorSerde::Absent => "removed (was absent before the interrupted run)".into(),
        PriorSerde::File { .. } => "prior bytes restored".into(),
        PriorSerde::Symlink { target } => format!("prior symlink → {target} restored"),
    };
    match live {
        // the mutation landed intact (or the intended removal held)
        Some(l) if l == intended => Recovery::Restore(restore_what()),
        // live IS the prior: the mutation never landed
        Some(l) if Some(l) == prior_id => Recovery::Unchanged,
        Some(_) => Recovery::Keep("changed since the interrupted run — your edit stands".into()),
        // absent now: a landed removal (intended REMOVED) or a never-
        // landed creation — either way the prior comes back
        None if prior_id.is_some() => Recovery::Restore(restore_what()),
        None => Recovery::Unchanged,
    }
}

fn restore(dest_dir: &Dir, dest_name: &Path, prior: &PriorSerde, home: &Dir) -> io::Result<()> {
    match prior {
        PriorSerde::Absent => {
            // remove_file never removes a directory; a dest that grew
            // into one errors here (ENOTDIR) and the journal entry is
            // retained — recovery data is never dropped on a failed
            // removal (0029 §4)
            match dest_dir.remove_file(dest_name) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        PriorSerde::File { hash, mode } => {
            let bytes = home.read(prior_blob_rel(hash))?;
            // the mode rides the write (0027 §6): temp → exact mode →
            // fsync → rename, so a restored 0600 secret never exists
            // at a wider mode, not even for the rename's instant
            gripsack_fs::atomic_write_with_mode(dest_dir, dest_name, &bytes, *mode)?
        }
        PriorSerde::Symlink { target } => {
            gripsack_fs::symlink_replace(dest_dir, dest_name, Path::new(target))?;
        }
    }
    Ok(())
}
