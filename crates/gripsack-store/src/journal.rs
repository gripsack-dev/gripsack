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

use crate::fs;
use gripsack_fs::Dir;
use std::io;
use std::path::{Path, PathBuf};

/// A destination's state before a journaled mutation.
#[derive(Debug, Clone, PartialEq)]
pub enum Prior {
    /// Nothing was there; recovery removes what the deploy wrote.
    Absent,
    /// A regular file; its bytes are in the prior blob store under
    /// `hash`.
    File { hash: String },
    /// A symlink; recovery recreates it pointing at `target`.
    Symlink { target: String },
}

/// One journal entry, one JSON file in the journal dir.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The destination path, as written (post `~` expansion).
    pub dest: String,
    pub prior: PriorSerde,
    /// Post-mutation identity: the content hash (canonical) or the
    /// link target. None until the mutation is known to have landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// Wire shape of [`Prior`] — the blob-store hash rides along for
/// files so recovery needs no re-derivation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PriorSerde {
    Absent,
    File { hash: String },
    Symlink { target: String },
}

impl From<&Prior> for PriorSerde {
    fn from(p: &Prior) -> Self {
        match p {
            Prior::Absent => PriorSerde::Absent,
            Prior::File { hash } => PriorSerde::File { hash: hash.clone() },
            Prior::Symlink { target } => PriorSerde::Symlink {
                target: target.clone(),
            },
        }
    }
}

/// Where the journal lives.
pub fn dir(home: &Path) -> PathBuf {
    home.join("journal")
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
        let hash = fs::store_prior_blob_in(home, &bytes)?;
        return Ok(Prior::File { hash });
    }
    // directories/fifos/devices are refused by deploy's guards; if
    // one is here anyway, treat it as absent — recovery will not
    // delete a directory through this path (remove_file only)
    Ok(Prior::Absent)
}

/// Record the intent to mutate `dest` whose prior state is `prior`:
/// the entry must be durable BEFORE the mutation lands.
pub fn record(home: &Dir, dest: &Path, prior: &Prior) -> io::Result<()> {
    let entry = Entry {
        dest: dest.to_string_lossy().into_owned(),
        prior: prior.into(),
        after: None,
    };
    gripsack_fs::atomic_write(
        home,
        &entry_rel(dest),
        serde_json::to_string(&entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )
}

/// Note the post-mutation identity, so recovery can tell "still the
/// deployed bytes" (restore prior) from "someone edited it since"
/// (leave it alone).
pub fn mark_after(home: &Dir, dest: &Path, after: &str) -> io::Result<()> {
    let path = entry_rel(dest);
    let mut entry: Entry = serde_json::from_slice(&home.read(&path)?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    entry.after = Some(after.to_string());
    gripsack_fs::atomic_write(
        home,
        &path,
        serde_json::to_string(&entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )
}

/// The run marker: which generation this journal's entries belong to.
/// Written before the first mutation; the flip makes it true.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RunMarker {
    target_generation: u64,
}

/// Declare the generation this run is building, BEFORE any mutation:
/// recovery compares it against `current` — a crash between the flip
/// and journal cleanup must NOT restore priors the committed
/// generation now owns (the post-commit window, review finding 5.1).
pub fn begin_run(home: &Dir, target_generation: u64) -> io::Result<()> {
    let marker = RunMarker { target_generation };
    gripsack_fs::atomic_write(
        home,
        &run_marker_rel(),
        serde_json::to_string(&marker)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )
}

/// The run completed and the generation flipped: nothing left to
/// recover. Entries, stragglers, and the run marker are gone.
pub fn commit_run(home: &Dir) -> io::Result<()> {
    let entries = match home.read_dir("journal") {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let name = entry?.file_name();
        let rel = Path::new("journal").join(&name);
        if rel.extension().is_some_and(|e| e == "json") {
            home.remove_file(&rel)?;
        }
    }
    // deletions are durable before we return: a power loss after
    // cleanup must not resurrect entries whose sibling run-marker
    // deletion WAS durable — reconcile would then read no marker and
    // restore a committed generation's priors (review: fsync the
    // journal directory after deletion)
    gripsack_fs::fsync_dir(home, Path::new("journal"))
}

/// The run ended without mutating anything (satisfied, empty graph):
/// the marker declared by `begin_run` must not linger — a stale
/// marker with no entries is harmless but noisy, and a marker whose
/// target generation is later than `current` would misread the NEXT
/// crash window.
pub fn end_run(home: &Dir) -> io::Result<()> {
    let _ = home.remove_file(run_marker_rel());
    let _ = gripsack_fs::fsync_dir(home, Path::new("journal"));
    Ok(())
}

/// What an uncommitted entry asks for, once resolved against the
/// current filesystem.
enum Recovery {
    /// Restore the prior state; `String` describes it for the report.
    Restore(String),
    /// The destination drifted after the crash — the user's now.
    Keep(String),
}

/// Resolve uncommitted journal entries from an interrupted run:
/// committed runs (the flip landed) are cleaned up, their content
/// stands; uncommitted runs are restored to their priors. Returns one
/// human line per decision for the apply report. Must run under the
/// lifecycle lock.
pub fn reconcile(home: &Dir) -> io::Result<Vec<String>> {
    let Some(entries) = read_uncommitted(home)? else {
        return Ok(Vec::new());
    };
    // the commit decision: a run whose target generation is current
    // (or older than current — later runs happened) COMMITTED. Only a
    // run whose target is still ahead restored.
    let committed = run_marker(home)?.is_some_and(|target| {
        crate::generations::current_in(home).is_some_and(|current| current >= target)
    });
    let mut lines = Vec::new();
    if committed {
        for (path, _) in &entries {
            home.remove_file(path)?;
        }
        let _ = home.remove_file(run_marker_rel());
        lines.push(
            "interrupted run's generation had already activated — journal \
             cleared, deployed state stands"
                .to_string(),
        );
        return Ok(lines);
    }
    for (path, entry) in entries {
        let dest = PathBuf::from(&entry.dest);
        // the drift check and the restore share ONE pinned parent
        // inode: a parent symlink swapped between decide() and
        // restore() cannot redirect the recovery write (plan/0021)
        let (dest_dir, dest_name) = dest_capability(&dest)?;
        match decide(&dest_dir, &dest_name, &entry) {
            Recovery::Restore(what) => {
                restore(&dest_dir, &dest_name, &entry.prior, home)?;
                lines.push(format!("recovered {}: {what}", entry.dest));
            }
            Recovery::Keep(why) => {
                lines.push(format!("kept {}: {why}", entry.dest));
            }
        }
        home.remove_file(&path)?;
    }
    let _ = home.remove_file(run_marker_rel());
    Ok(lines)
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

fn run_marker(home: &Dir) -> io::Result<Option<u64>> {
    let Ok(bytes) = home.read(run_marker_rel()) else {
        return Ok(None);
    };
    serde_json::from_slice::<RunMarker>(&bytes)
        .map(|m| Some(m.target_generation))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
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
fn decide(dest_dir: &Dir, dest_name: &Path, entry: &Entry) -> Recovery {
    let Some(after) = &entry.after else {
        return Recovery::Restore(match &entry.prior {
            PriorSerde::Absent => "removed (was absent before the interrupted run)".into(),
            PriorSerde::File { .. } => "prior bytes restored".into(),
            PriorSerde::Symlink { target } => format!("prior symlink → {target} restored"),
        });
    };
    let current = if dest_dir
        .symlink_metadata(dest_name)
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        dest_dir
            .read_link_contents(dest_name)
            .map(|t| t.to_string_lossy().into_owned())
            .ok()
    } else {
        dest_dir
            .read(dest_name)
            .ok()
            .map(|b| crate::hash::hex_sha256(&b))
    };
    match current {
        Some(id) if id == *after => {
            Recovery::Restore("still the interrupted run's content — prior state restored".into())
        }
        Some(_) => Recovery::Keep("changed since the interrupted run — your edit stands".into()),
        None => Recovery::Restore("prior state restored".into()),
    }
}

fn restore(dest_dir: &Dir, dest_name: &Path, prior: &PriorSerde, home: &Dir) -> io::Result<()> {
    match prior {
        PriorSerde::Absent => {
            // remove_file never removes a directory; a dest that grew
            // into one is left for the drift guard's Keep path
            let _ = dest_dir.remove_file(dest_name);
        }
        PriorSerde::File { hash } => {
            let bytes = home.read(fs::prior_blob_rel(hash))?;
            gripsack_fs::atomic_write(dest_dir, dest_name, &bytes)?;
        }
        PriorSerde::Symlink { target } => {
            gripsack_fs::symlink_replace(dest_dir, dest_name, Path::new(target))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
        record(&cap(&home), &dest, &prior).unwrap();
        // simulate the crash: mutation never happened, no after, no
        // commit — the file still holds the prior bytes

        let lines = reconcile(&cap(&home)).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("recovered"));
        // restore is idempotent for an unwritten dest
        assert_eq!(std::fs::read(&dest).unwrap(), b"user stuff\n");
        // the entry is consumed either way
        assert!(reconcile(&cap(&home)).unwrap().is_empty());
    }

    #[test]
    fn crash_after_write_restores_prior_bytes() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();

        let prior = capture_at(&dest, &cap(&home));
        record(&cap(&home), &dest, &prior).unwrap();
        std::fs::write(&dest, b"deployed half-run content\n").unwrap();
        mark_after(
            &cap(&home),
            &dest,
            &crate::hash::hex_sha256(b"deployed half-run content\n"),
        )
        .unwrap();
        // crash: no commit_run

        let lines = reconcile(&cap(&home)).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
        assert!(lines[0].contains("recovered"));
    }

    #[test]
    fn user_edit_after_crash_wins() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();

        let prior = capture_at(&dest, &cap(&home));
        record(&cap(&home), &dest, &prior).unwrap();
        std::fs::write(&dest, b"deployed\n").unwrap();
        mark_after(&cap(&home), &dest, &crate::hash::hex_sha256(b"deployed\n")).unwrap();
        // the user edits the file AFTER the crash, before the next run
        std::fs::write(&dest, b"my own edit\n").unwrap();

        let lines = reconcile(&cap(&home)).unwrap();
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
        record(&cap(&home), &fresh, &prior).unwrap();
        std::fs::write(&fresh, b"crashed write\n").unwrap();
        mark_after(
            &cap(&home),
            &fresh,
            &crate::hash::hex_sha256(b"crashed write\n"),
        )
        .unwrap();

        std::os::unix::fs::symlink("/original/target", &link).unwrap();
        let link_prior = capture_at(&link, &cap(&home));
        record(&cap(&home), &link, &link_prior).unwrap();
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("/deployed/target", &link).unwrap();
        mark_after(&cap(&home), &link, "/deployed/target").unwrap();

        reconcile(&cap(&home)).unwrap();
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

        begin_run(&cap(&home), 1).unwrap();
        let prior = capture_at(&dest, &cap(&home));
        record(&cap(&home), &dest, &prior).unwrap();
        std::fs::write(&dest, b"deployed\n").unwrap();
        mark_after(&cap(&home), &dest, &crate::hash::hex_sha256(b"deployed\n")).unwrap();
        // the flip: generation 1 becomes current; commit_run never ran
        let manifest = crate::generations::Generation {
            number: 1,
            modules: Default::default(),
        };
        crate::generations::write_manifest(home.path(), &manifest).unwrap();
        crate::generations::flip(home.path(), 1).unwrap();

        let lines = reconcile(&cap(&home)).unwrap();
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
        begin_run(&cap(&home), 1).unwrap();
        let prior = capture_at(&dest, &cap(&home));
        record(&cap(&home), &dest, &prior).unwrap();
        std::fs::write(&dest, b"half\n").unwrap();
        mark_after(&cap(&home), &dest, &crate::hash::hex_sha256(b"half\n")).unwrap();

        reconcile(&cap(&home)).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
    }

    #[test]
    fn malformed_entries_fail_closed_into_quarantine() {
        // review 5.2: corrupt recovery metadata is quarantined and
        // BLOCKS mutation — never silently deleted
        let home = home();
        std::fs::create_dir_all(dir(home.path())).unwrap();
        std::fs::write(dir(home.path()).join("deadbeef.json"), b"{ not json").unwrap();
        let err = reconcile(&cap(&home)).unwrap_err();
        assert!(err.to_string().contains("quarantine"), "{err}");
        assert!(dir(home.path()).join("quarantine/deadbeef.json").exists());
    }

    #[test]
    fn commit_run_clears_the_window() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();
        let prior = capture_at(&dest, &cap(&home));
        record(&cap(&home), &dest, &prior).unwrap();
        std::fs::write(&dest, b"new\n").unwrap();
        mark_after(&cap(&home), &dest, &crate::hash::hex_sha256(b"new\n")).unwrap();

        commit_run(&cap(&home)).unwrap();
        // the flip happened: recovery must NOT undo the deploy
        assert!(reconcile(&cap(&home)).unwrap().is_empty());
        assert_eq!(std::fs::read(&dest).unwrap(), b"new\n");
    }
}
