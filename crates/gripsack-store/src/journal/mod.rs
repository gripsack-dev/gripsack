//! The deploy journal (plan/0019): crash recovery for destination
//! mutations that happen BEFORE the generation flip.
//!
//! `apply` mutates real destinations (owned links, tracked copies,
//! templates, merge blocks) while the current generation still points
//! at the old state. An in-process failure compensates via the
//! run-level rollback — but a `kill -9` or power loss skips it, and
//! the filesystem is left between generations with no record of what
//! to undo.
//!
//! The journal closes that window:
//!
//! 1. **record** — before each mutation, the destination's prior
//!    state is captured (file bytes into the prior blob store, or a
//!    symlink target, or `Absent`) and an entry lands in
//!    `$GRIPSACK_HOME/journal/`, fsync'd, marked uncommitted.
//! 2. **after** — once the mutation lands, the entry gains the
//!    post-mutation identity (content hash or link target).
//! 3. **commit_run** — after the flip succeeds, every entry is
//!    deleted: the generation now owns the truth.
//! 4. **reconcile** — the next run (under the lifecycle lock, before
//!    deploying anything) restores every uncommitted entry to its
//!    prior state. The drift guard applies: when the entry knows the
//!    post-mutation identity and the destination no longer matches
//!    it, someone touched the file after the crash — their edit wins,
//!    the entry is dropped with a warning. Never delete user edits.
//!
//! An entry without an `after` (crashed between record and mutation,
//! or between mutation and the after-mark) restores unconditionally —
//! the same choice the in-process rollback makes on failure.
//! 4. **reconcile** — see `recover` (the drift guard lives there).

pub mod marker;
pub(crate) mod recover;

pub use marker::{RunOp, begin_run, commit_run, end_run};
pub use recover::reconcile;

use gripsack_fs::Dir;
use std::io;
use std::path::{Path, PathBuf};

/// A destination's state before a journaled mutation.
#[derive(Debug, Clone, PartialEq)]
pub enum Prior {
    /// Nothing was there; recovery removes what the deploy wrote.
    Absent,
    /// A regular file; its bytes are in the prior blob store under
    /// `hash`. `mode` is the original Unix mode (0027 §6 — recovery
    /// recreates the file exactly: a 0600 secret must not come back
    /// 0644&umask, an 0755 script must stay executable; also the
    /// file→symlink→crash path, where the live object at recovery is
    /// a link and mode preservation by copy is impossible).
    File { hash: String, mode: Option<u32> },
    /// A symlink; recovery recreates it pointing at `target`.
    Symlink { target: String },
}

/// One journal entry, one JSON file in the journal dir.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The destination path, as written (post `~` expansion).
    pub dest: String,
    pub prior: PriorSerde,
    /// The INTENDED post-mutation identity (0026 §6): canonical
    /// content hash, link target, or the REMOVED sentinel — recorded
    /// BEFORE the mutation, so recovery makes a three-way decision
    /// (live == intended → restore prior; live == prior → the
    /// mutation never landed; else → someone's edit, keep it).
    pub after: String,
}

/// Wire shape of [`Prior`] — the blob-store hash rides along for
/// files so recovery needs no re-derivation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PriorSerde {
    Absent,
    File {
        hash: String,
        /// Original Unix mode; absent only in pre-0.24 entries.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<u32>,
    },
    Symlink {
        target: String,
    },
}

impl From<&Prior> for PriorSerde {
    fn from(p: &Prior) -> Self {
        match p {
            Prior::Absent => PriorSerde::Absent,
            Prior::File { hash, mode } => PriorSerde::File {
                hash: hash.clone(),
                mode: *mode,
            },
            Prior::Symlink { target } => PriorSerde::Symlink {
                target: target.clone(),
            },
        }
    }
}

/// The `after` identity recorded for a journaled REMOVAL: nothing
/// should be there. Distinctive enough to never collide with a real
/// content hash or link target — a destination that exists again at
/// reconcile reads as user content and is kept (0025 §B).
pub const REMOVED: &str = "gripsack:removed";

/// Where the journal lives.
pub fn dir(home: &Path) -> PathBuf {
    home.join("journal")
}

/// A prior blob's path relative to the home capability:
/// `prior/<sha256>`.
pub fn prior_blob_rel(sha: &str) -> PathBuf {
    Path::new("prior").join(sha)
}

/// Store pre-take-over bytes content-addressed (0015 §4): returns the
/// sha256 the manifest references. Dedup is the point — priors are
/// small, and identical originals share one blob. Written through the
/// home capability (plan/0021).
pub fn store_prior_blob_in(home: &Dir, bytes: &[u8]) -> io::Result<String> {
    let sha = crate::hash::hex_sha256(bytes);
    let rel = prior_blob_rel(&sha);
    // symlink_metadata does not follow links: a planted `prior/<sha>`
    // symlink would skip the write and later restore THROUGH it. Skip
    // only a real regular file; anything else is replaced by the
    // atomic rename.
    let present = home
        .symlink_metadata(&rel)
        .map(|m| m.is_file())
        .unwrap_or(false);
    if present {
        // trust nothing by name (0029 §8): the blob exists — prove the
        // bytes or quarantine the impostor aside and write the truth
        let existing = home.read(&rel)?;
        if crate::hash::hex_sha256(&existing) != sha {
            let aside = Path::new("prior").join(format!("{sha}.corrupt"));
            home.rename(&rel, home, &aside)?;
            gripsack_fs::atomic_write(home, &rel, bytes)?;
        }
    } else {
        gripsack_fs::atomic_write(home, &rel, bytes)?;
    }
    Ok(sha)
}

/// An entry's path relative to the home capability: the journal's
/// own files (entries, run marker, quarantine, prior blobs) never
/// leave `$GRIPSACK_HOME`, so they are named relative to the `Dir`
/// every journal function takes (plan/0021).
fn entry_rel(dest: &Path) -> PathBuf {
    Path::new("journal").join(format!(
        "{}.json",
        crate::hash::hex_sha256(dest.to_string_lossy().as_bytes(),)
    ))
}

fn run_marker_rel() -> PathBuf {
    Path::new("journal").join("run.json")
}

/// Capture a destination's current state through its pinned parent
/// capability (plan/0021): the capture, the journaled write, and the
/// mark-after all name the SAME parent inode — a swapped path
/// component cannot redirect the mutation the journal is protecting.
/// File bytes back up into the prior blob store under `home`.
/// `dest` is the display/record form (absolute); `dest_dir` +
/// `dest_name` are the access path. Call immediately before the
/// mutation; the capture and the write race nothing (the lifecycle
/// lock serializes runs).
pub fn capture(dest_dir: &Dir, dest_name: &Path, dest: &Path, home: &Dir) -> io::Result<Prior> {
    let meta = match dest_dir.symlink_metadata(dest_name) {
        Ok(meta) => meta,
        // only NotFound means absent — a permission error or I/O
        // failure recorded as Absent would make recovery REMOVE a
        // destination it could not even read (review finding)
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Prior::Absent),
        Err(e) => {
            return Err(io::Error::other(format!(
                "cannot inspect {} to journal its prior state: {e}",
                dest.display()
            )));
        }
    };
    if meta.file_type().is_symlink() {
        let target = dest_dir.read_link_contents(dest_name)?;
        let Some(target) = target.to_str() else {
            // a non-UTF-8 target lossily recorded would re-create a
            // DIFFERENT link on recovery — refuse the mutation
            // instead of corrupting it on undo
            return Err(io::Error::other(format!(
                "{} is a symlink with a non-UTF-8 target ({} bytes) — gripsack \
                 cannot journal it for recovery; remove or adopt it by hand",
                dest.display(),
                target.as_os_str().len()
            )));
        };
        return Ok(Prior::Symlink {
            target: target.to_string(),
        });
    }
    if meta.is_file() {
        let bytes = dest_dir.read(dest_name)?;
        let hash = store_prior_blob_in(home, &bytes)?;
        #[cfg(unix)]
        let mode = {
            use gripsack_fs::cap_std::fs::MetadataExt;
            Some(meta.mode() & 0o777)
        };
        #[cfg(not(unix))]
        let mode = None;
        return Ok(Prior::File { hash, mode });
    }
    // directories/fifos/devices are refused by deploy's guards; if
    // one is here anyway, treat it as absent — recovery will not
    // delete a directory through this path (remove_file only)
    Ok(Prior::Absent)
}

/// Record the intent to mutate `dest`: prior state AND the intended
/// post-mutation identity, durable BEFORE the mutation lands
/// (0026 §6 — persisting intent up front closes the window where a
/// post-crash user edit was indistinguishable from the mutation).
pub fn record(home: &Dir, dest: &Path, prior: &Prior, after: &str) -> io::Result<()> {
    let entry = Entry {
        dest: dest.to_string_lossy().into_owned(),
        prior: prior.into(),
        after: after.to_string(),
    };
    gripsack_fs::atomic_write(
        home,
        &entry_rel(dest),
        serde_json::to_string(&entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )
}

/// Open a destination's parent as a capability, returning it with the
/// destination's bare file name. `open_or_create` because a crashed
/// run's destination may sit under parents the mutation itself was
/// about to create.
fn dest_capability(dest: &Path) -> io::Result<(Dir, PathBuf)> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let dir = gripsack_fs::open_or_create(parent)?;
    Ok((dir, PathBuf::from(dest.file_name().unwrap_or_default())))
}

/// Entries on disk, with the run marker if any. Malformed recovery
/// metadata FAILS CLOSED: the file moves to `journal/quarantine/`
/// and reconcile errors — the one structure responsible for
/// recovering user files must never be shrugged off as archaeology
/// (review finding 5.2). Inspect and delete the quarantine to
/// proceed.
fn read_uncommitted(home: &Dir) -> io::Result<Option<Vec<(PathBuf, Entry)>>> {
    let entries = match home.read_dir("journal") {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    let mut quarantined = 0usize;
    for entry in entries {
        let name = entry?.file_name();
        if name == "run.json" {
            continue;
        }
        let rel = Path::new("journal").join(&name);
        if rel.extension().is_some_and(|e| e != "json") {
            continue;
        }
        match home.read(&rel).and_then(|b| {
            serde_json::from_slice::<Entry>(&b)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }) {
            Ok(parsed) => out.push((rel, parsed)),
            Err(_) => {
                let quarantine = Path::new("journal").join("quarantine");
                home.create_dir_all(&quarantine)?;
                let _ = home.rename(&rel, home, quarantine.join(&name));
                quarantined += 1;
            }
        }
    }
    if quarantined > 0 {
        return Err(io::Error::other(format!(
            "{} journal entr{} could not be parsed — moved to journal/quarantine \
             under $GRIPSACK_HOME; inspect and remove them to continue \
             (recovery metadata is never ignored)",
            quarantined,
            if quarantined == 1 { "y" } else { "ies" },
        )));
    }
    Ok(Some(out))
}

/// The drift guard, same philosophy as everywhere else: a known
/// `after` that no longer matches means the file changed since the
/// crash — never delete user edits. Reads go through the destination
/// capability reconcile pinned.
/// The destination's live identity in the journal's terms: link
/// target for symlinks, canonical content hash for files, None when
/// absent. (canonical_bytes_hash — the identity deploy records; a
/// raw-sha256 comparison here was the latent drift-guard bug the
/// 0025 crash-window e2e exposed.)
pub fn live_identity(dest_dir: &Dir, dest_name: &Path) -> io::Result<Option<String>> {
    match dest_dir.symlink_metadata(dest_name) {
        Ok(meta) if meta.file_type().is_symlink() => Ok(Some(
            dest_dir
                .read_link_contents(dest_name)?
                .to_string_lossy()
                .into_owned(),
        )),
        Ok(meta) => {
            // exec-aware (0030 §H3): the journal's file identity
            // matches what deploy records for a tracked copy
            use gripsack_fs::cap_std::fs::MetadataExt;
            let exec = meta.mode() & 0o111 != 0;
            Ok(Some(crate::hash::canonical_bytes_identity(
                &dest_dir.read(dest_name)?,
                exec,
            )))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The prior's identity in the same terms (recomputed from the blob;
/// the blob's own hash is the raw sha256 used for addressing).
fn prior_identity(prior: &PriorSerde, home: &Dir) -> io::Result<Option<String>> {
    match prior {
        PriorSerde::Absent => Ok(None),
        PriorSerde::Symlink { target } => Ok(Some(target.clone())),
        PriorSerde::File { hash, mode } => Ok(Some(crate::hash::canonical_bytes_identity(
            &home.read(prior_blob_rel(hash))?,
            mode.is_some_and(|m| m & 0o111 != 0),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::marker::*;
    use super::recover::*;
    use super::*;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    /// The home capability the journal API takes now (plan/0021) —
    /// opened on the temp home; assertions below are unchanged.
    fn cap(home: &tempfile::TempDir) -> Dir {
        gripsack_fs::open_or_create(home.path()).expect("home capability")
    }

    /// Capture through the destination's pinned parent capability,
    /// as deploy's journaled mutations do (plan/0021).
    fn capture_at(dest: &Path, home: &Dir) -> Prior {
        let dir = gripsack_fs::open_or_create(dest.parent().unwrap()).unwrap();
        capture(&dir, Path::new(dest.file_name().unwrap()), dest, home).unwrap()
    }

    #[test]
    fn crash_between_record_and_write_restores_prior() {
        let home = home();
        let dest = home.path().join("rc/.bashrc");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"user stuff\n").unwrap();

        let prior = capture_at(&dest, &cap(&home));
        // the intended post-state is recorded up front (0026 §6)
        record(&cap(&home), &dest, &prior, "intended-hash").unwrap();
        // simulate the crash: mutation never happened, no commit —
        // the file still holds the prior bytes

        let lines = reconcile(&cap(&home), home.path()).unwrap();
        assert_eq!(lines.len(), 1);
        // live IS the prior: the mutation never landed, so there is
        // nothing to restore — the entry is still consumed
        assert!(lines[0].contains("unchanged"), "{lines:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"user stuff\n");
        assert!(reconcile(&cap(&home), home.path()).unwrap().is_empty());
    }

    #[test]
    fn crash_after_write_restores_prior_bytes() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();

        let prior = capture_at(&dest, &cap(&home));
        record(
            &cap(&home),
            &dest,
            &prior,
            &crate::hash::canonical_bytes_hash(b"deployed half-run content\n"),
        )
        .unwrap();
        std::fs::write(&dest, b"deployed half-run content\n").unwrap();
        // crash: no commit_run

        let lines = reconcile(&cap(&home), home.path()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
        assert!(lines[0].contains("recovered"));
    }

    #[test]
    fn user_edit_after_crash_wins() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();

        let prior = capture_at(&dest, &cap(&home));
        record(
            &cap(&home),
            &dest,
            &prior,
            &crate::hash::canonical_bytes_hash(b"deployed\n"),
        )
        .unwrap();
        std::fs::write(&dest, b"deployed\n").unwrap();
        // the user edits the file AFTER the crash, before the next run
        std::fs::write(&dest, b"my own edit\n").unwrap();

        let lines = reconcile(&cap(&home), home.path()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"my own edit\n");
        assert!(lines[0].contains("kept"));
    }

    #[test]
    fn absent_prior_and_symlink_prior_recover() {
        let home = home();
        let fresh = home.path().join("fresh");
        let link = home.path().join("link");

        let prior = capture_at(&fresh, &cap(&home));
        assert_eq!(prior, Prior::Absent);
        record(
            &cap(&home),
            &fresh,
            &prior,
            &crate::hash::canonical_bytes_hash(b"crashed write\n"),
        )
        .unwrap();
        std::fs::write(&fresh, b"crashed write\n").unwrap();

        std::os::unix::fs::symlink("/original/target", &link).unwrap();
        let link_prior = capture_at(&link, &cap(&home));
        record(&cap(&home), &link, &link_prior, "/deployed/target").unwrap();
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("/deployed/target", &link).unwrap();

        reconcile(&cap(&home), home.path()).unwrap();
        assert!(!fresh.exists(), "absent prior removes the crashed write");
        assert_eq!(
            std::fs::read_link(&link).unwrap().to_string_lossy(),
            "/original/target"
        );
    }

    #[test]
    fn crash_after_flip_but_before_cleanup_reads_committed() {
        // review 5.1: the crash lands between the flip and journal
        // cleanup. The run marker names the target generation and
        // `current` reached it — the deployed content STANDS.
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();

        begin_run(&cap(&home), None, 1, RunOp::Apply).unwrap();
        let prior = capture_at(&dest, &cap(&home));
        record(
            &cap(&home),
            &dest,
            &prior,
            &crate::hash::canonical_bytes_hash(b"deployed\n"),
        )
        .unwrap();
        std::fs::write(&dest, b"deployed\n").unwrap();
        // the flip: generation 1 becomes current; commit_run never ran
        let manifest = crate::generations::Generation {
            number: 1,
            modules: Default::default(),
        };
        crate::generations::write_manifest(&cap(&home), &manifest).unwrap();
        crate::generations::flip(&cap(&home), home.path(), 1).unwrap();

        let lines = reconcile(&cap(&home), home.path()).unwrap();
        assert!(
            lines.iter().any(|l| l.contains("already activated")),
            "{lines:?}"
        );
        // the committed generation's content is NOT rolled back
        assert_eq!(std::fs::read(&dest).unwrap(), b"deployed\n");
    }

    #[test]
    fn crash_before_flip_restores() {
        // the mirror case: the marker names generation 1 but current
        // is still nothing — restore the prior
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();
        begin_run(&cap(&home), None, 1, RunOp::Apply).unwrap();
        let prior = capture_at(&dest, &cap(&home));
        record(
            &cap(&home),
            &dest,
            &prior,
            &crate::hash::canonical_bytes_hash(b"half\n"),
        )
        .unwrap();
        std::fs::write(&dest, b"half\n").unwrap();

        reconcile(&cap(&home), home.path()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
    }

    #[test]
    fn malformed_entries_fail_closed_into_quarantine() {
        // review 5.2: corrupt recovery metadata is quarantined and
        // BLOCKS mutation — never silently deleted
        let home = home();
        std::fs::create_dir_all(dir(home.path())).unwrap();
        std::fs::write(dir(home.path()).join("deadbeef.json"), b"{ not json").unwrap();
        let err = reconcile(&cap(&home), home.path()).unwrap_err();
        assert!(err.to_string().contains("quarantine"), "{err}");
        assert!(dir(home.path()).join("quarantine/deadbeef.json").exists());
    }

    #[test]
    fn commit_run_clears_the_window() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();
        let prior = capture_at(&dest, &cap(&home));
        record(
            &cap(&home),
            &dest,
            &prior,
            &crate::hash::canonical_bytes_hash(b"new\n"),
        )
        .unwrap();
        std::fs::write(&dest, b"new\n").unwrap();

        commit_run(&cap(&home)).unwrap();
        // the flip happened: recovery must NOT undo the deploy
        assert!(reconcile(&cap(&home), home.path()).unwrap().is_empty());
        assert_eq!(std::fs::read(&dest).unwrap(), b"new\n");
    }
    fn manifest(n: u64) -> crate::generations::Generation {
        crate::generations::Generation {
            number: n,
            modules: Default::default(),
        }
    }

    #[test]
    fn recovery_restores_the_exact_mode() {
        // 0027 §6: a 0600 secret replaced by a symlink mid-run, then a
        // crash — the live object at recovery is a LINK, so mode
        // preservation by copy is impossible; the mode must come from
        // the journal entry itself
        use std::os::unix::fs::PermissionsExt;
        let home = home();
        let dest = home.path().join("secret");
        std::fs::write(&dest, b"hunter2\n").unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600)).unwrap();

        let prior = capture_at(&dest, &cap(&home));
        record(&cap(&home), &dest, &prior, "/store/x").unwrap();
        // the mutation: dest becomes an owned symlink
        std::fs::remove_file(&dest).unwrap();
        std::os::unix::fs::symlink("/store/x", &dest).unwrap();
        // crash before commit

        reconcile(&cap(&home), home.path()).unwrap();
        let meta = std::fs::metadata(&dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hunter2\n");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn post_crash_edit_beats_the_landed_mutation() {
        // 0026 §6: intent is recorded BEFORE the mutation, so an edit
        // made after the crash is distinguishable from the mutation —
        // the user's bytes win even when the mutation fully landed
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();

        let prior = capture_at(&dest, &cap(&home));
        record(
            &cap(&home),
            &dest,
            &prior,
            &crate::hash::canonical_bytes_hash(b"deployed\n"),
        )
        .unwrap();
        std::fs::write(&dest, b"deployed\n").unwrap();
        // crash; THEN the user edits
        std::fs::write(&dest, b"post-crash edit\n").unwrap();

        let lines = reconcile(&cap(&home), home.path()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"post-crash edit\n");
        assert!(lines[0].contains("kept"), "{lines:?}");
    }

    #[test]
    fn crashed_rollback_restores_priors() {
        // 0025 §A: a rollback's target is OLDER than current — the
        // pre-0025 commit rule (current >= target) would have misread
        // a crashed rollback as committed and skipped restoration.
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"new\n").unwrap();
        crate::generations::write_manifest(&cap(&home), &manifest(2)).unwrap();
        crate::generations::write_manifest(&cap(&home), &manifest(3)).unwrap();
        crate::generations::flip(&cap(&home), home.path(), 3).unwrap();

        // rolling back 3 → 2: the restore lands, the flip never does
        begin_run(&cap(&home), Some(3), 2, RunOp::Rollback).unwrap();
        let prior = capture_at(&dest, &cap(&home));
        record(
            &cap(&home),
            &dest,
            &prior,
            &crate::hash::canonical_bytes_hash(b"old\n"),
        )
        .unwrap();
        std::fs::write(&dest, b"old\n").unwrap();
        // crash: current is still 3, target was 2 — uncommitted

        let lines = reconcile(&cap(&home), home.path()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new\n");
        assert!(lines[0].contains("recovered"), "{lines:?}");
    }

    #[test]
    fn completed_rollback_reads_committed() {
        // the flip landed (current back DOWN to the target) but
        // cleanup never ran: the restored content STANDS
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"new\n").unwrap();
        crate::generations::write_manifest(&cap(&home), &manifest(2)).unwrap();
        crate::generations::write_manifest(&cap(&home), &manifest(3)).unwrap();
        crate::generations::flip(&cap(&home), home.path(), 3).unwrap();

        begin_run(&cap(&home), Some(3), 2, RunOp::Rollback).unwrap();
        let prior = capture_at(&dest, &cap(&home));
        record(
            &cap(&home),
            &dest,
            &prior,
            &crate::hash::canonical_bytes_hash(b"old\n"),
        )
        .unwrap();
        std::fs::write(&dest, b"old\n").unwrap();
        crate::generations::flip(&cap(&home), home.path(), 2).unwrap();
        // crash between the flip and commit_run

        let lines = reconcile(&cap(&home), home.path()).unwrap();
        assert!(
            lines.iter().any(|l| l.contains("already activated")),
            "{lines:?}"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
    }
}

/// The exhaustive state-machine model of this protocol
/// (plan/0028) — see the module's own documentation.
#[cfg(test)]
mod model;
