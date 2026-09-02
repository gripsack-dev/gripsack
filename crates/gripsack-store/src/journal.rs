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

fn entry_path(home: &Path, dest: &Path) -> PathBuf {
    dir(home).join(format!(
        "{}.json",
        crate::hash::hex_sha256(dest.to_string_lossy().as_bytes(),)
    ))
}

/// Capture a destination's current state, backing file bytes up into
/// the prior blob store. Call immediately before the mutation; the
/// capture and the write race nothing (the lifecycle lock serializes
/// runs).
pub fn capture(dest: &Path, home: &Path) -> io::Result<Prior> {
    let meta = match std::fs::symlink_metadata(dest) {
        Ok(meta) => meta,
        Err(_) => return Ok(Prior::Absent),
    };
    if meta.file_type().is_symlink() {
        return Ok(Prior::Symlink {
            target: std::fs::read_link(dest)?.to_string_lossy().into_owned(),
        });
    }
    if meta.is_file() {
        let bytes = std::fs::read(dest)?;
        let hash = fs::store_prior_blob(home, &bytes)?;
        return Ok(Prior::File { hash });
    }
    // directories/fifos/devices are refused by deploy's guards; if
    // one is here anyway, treat it as absent — recovery will not
    // delete a directory through this path (remove_file only)
    Ok(Prior::Absent)
}

/// Record the intent to mutate `dest` whose prior state is `prior`:
/// the entry must be durable BEFORE the mutation lands.
pub fn record(home: &Path, dest: &Path, prior: &Prior) -> io::Result<()> {
    let entry = Entry {
        dest: dest.to_string_lossy().into_owned(),
        prior: prior.into(),
        after: None,
    };
    fs::atomic_write(
        &entry_path(home, dest),
        serde_json::to_string(&entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )
}

/// Note the post-mutation identity, so recovery can tell "still the
/// deployed bytes" (restore prior) from "someone edited it since"
/// (leave it alone).
pub fn mark_after(home: &Path, dest: &Path, after: &str) -> io::Result<()> {
    let path = entry_path(home, dest);
    let mut entry: Entry = serde_json::from_slice(&std::fs::read(&path)?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    entry.after = Some(after.to_string());
    fs::atomic_write(
        &path,
        serde_json::to_string(&entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )
}

/// The run completed and the generation flipped: nothing left to
/// recover. Entries from this run and any stragglers are gone.
pub fn commit_run(home: &Path) -> io::Result<()> {
    let dir = dir(home);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            std::fs::remove_file(&path)?;
        }
    }
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

/// Restore every uncommitted entry to its prior state (the run that
/// wrote them never flipped a generation). Returns one human line per
/// entry for the apply report. Must run under the lifecycle lock.
pub fn reconcile(home: &Path) -> io::Result<Vec<String>> {
    let Some(entries) = read_uncommitted(home)? else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    for (path, entry) in entries {
        let dest = PathBuf::from(&entry.dest);
        match decide(&dest, &entry) {
            Recovery::Restore(what) => {
                restore(&dest, &entry.prior, home)?;
                lines.push(format!("recovered {}: {what}", entry.dest));
            }
            Recovery::Keep(why) => {
                lines.push(format!("kept {}: {why}", entry.dest));
            }
        }
        std::fs::remove_file(&path)?;
    }
    Ok(lines)
}

fn read_uncommitted(home: &Path) -> io::Result<Option<Vec<(PathBuf, Entry)>>> {
    let dir = dir(home);
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e != "json") {
            continue;
        }
        // an unreadable entry is archaeology, not a crash window —
        // drop it rather than block every future apply
        let Ok(bytes) = std::fs::read(&path) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        match serde_json::from_slice::<Entry>(&bytes) {
            Ok(parsed) => out.push((path, parsed)),
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    Ok(Some(out))
}

/// The drift guard, same philosophy as everywhere else: a known
/// `after` that no longer matches means the file changed since the
/// crash — never delete user edits.
fn decide(dest: &Path, entry: &Entry) -> Recovery {
    let Some(after) = &entry.after else {
        return Recovery::Restore(match &entry.prior {
            PriorSerde::Absent => "removed (was absent before the interrupted run)".into(),
            PriorSerde::File { .. } => "prior bytes restored".into(),
            PriorSerde::Symlink { target } => format!("prior symlink → {target} restored"),
        });
    };
    let current = if dest
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        std::fs::read_link(dest)
            .map(|t| t.to_string_lossy().into_owned())
            .ok()
    } else {
        std::fs::read(dest)
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

fn restore(dest: &Path, prior: &PriorSerde, home: &Path) -> io::Result<()> {
    match prior {
        PriorSerde::Absent => {
            // remove_file never removes a directory; a dest that grew
            // into one is left for the drift guard's Keep path
            let _ = std::fs::remove_file(dest);
        }
        PriorSerde::File { hash } => {
            let bytes = std::fs::read(fs::prior_blob_path(home, hash))?;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            fs::atomic_write(dest, &bytes)?;
        }
        PriorSerde::Symlink { target } => {
            fs::symlink_replace(dest, Path::new(target))?;
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

    #[test]
    fn crash_between_record_and_write_restores_prior() {
        let home = home();
        let dest = home.path().join("rc/.bashrc");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"user stuff\n").unwrap();

        let prior = capture(&dest, home.path()).unwrap();
        record(home.path(), &dest, &prior).unwrap();
        // simulate the crash: mutation never happened, no after, no
        // commit — the file still holds the prior bytes

        let lines = reconcile(home.path()).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("recovered"));
        // restore is idempotent for an unwritten dest
        assert_eq!(std::fs::read(&dest).unwrap(), b"user stuff\n");
        // the entry is consumed either way
        assert!(reconcile(home.path()).unwrap().is_empty());
    }

    #[test]
    fn crash_after_write_restores_prior_bytes() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();

        let prior = capture(&dest, home.path()).unwrap();
        record(home.path(), &dest, &prior).unwrap();
        std::fs::write(&dest, b"deployed half-run content\n").unwrap();
        mark_after(
            home.path(),
            &dest,
            &crate::hash::hex_sha256(b"deployed half-run content\n"),
        )
        .unwrap();
        // crash: no commit_run

        let lines = reconcile(home.path()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
        assert!(lines[0].contains("recovered"));
    }

    #[test]
    fn user_edit_after_crash_wins() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();

        let prior = capture(&dest, home.path()).unwrap();
        record(home.path(), &dest, &prior).unwrap();
        std::fs::write(&dest, b"deployed\n").unwrap();
        mark_after(home.path(), &dest, &crate::hash::hex_sha256(b"deployed\n")).unwrap();
        // the user edits the file AFTER the crash, before the next run
        std::fs::write(&dest, b"my own edit\n").unwrap();

        let lines = reconcile(home.path()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"my own edit\n");
        assert!(lines[0].contains("kept"));
    }

    #[test]
    fn absent_prior_and_symlink_prior_recover() {
        let home = home();
        let fresh = home.path().join("fresh");
        let link = home.path().join("link");

        let prior = capture(&fresh, home.path()).unwrap();
        assert_eq!(prior, Prior::Absent);
        record(home.path(), &fresh, &prior).unwrap();
        std::fs::write(&fresh, b"crashed write\n").unwrap();
        mark_after(
            home.path(),
            &fresh,
            &crate::hash::hex_sha256(b"crashed write\n"),
        )
        .unwrap();

        std::os::unix::fs::symlink("/original/target", &link).unwrap();
        let link_prior = capture(&link, home.path()).unwrap();
        record(home.path(), &link, &link_prior).unwrap();
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("/deployed/target", &link).unwrap();
        mark_after(home.path(), &link, "/deployed/target").unwrap();

        reconcile(home.path()).unwrap();
        assert!(!fresh.exists(), "absent prior removes the crashed write");
        assert_eq!(
            std::fs::read_link(&link).unwrap().to_string_lossy(),
            "/original/target"
        );
    }

    #[test]
    fn commit_run_clears_the_window() {
        let home = home();
        let dest = home.path().join("config");
        std::fs::write(&dest, b"old\n").unwrap();
        let prior = capture(&dest, home.path()).unwrap();
        record(home.path(), &dest, &prior).unwrap();
        std::fs::write(&dest, b"new\n").unwrap();
        mark_after(home.path(), &dest, &crate::hash::hex_sha256(b"new\n")).unwrap();

        commit_run(home.path()).unwrap();
        // the flip happened: recovery must NOT undo the deploy
        assert!(reconcile(home.path()).unwrap().is_empty());
        assert_eq!(std::fs::read(&dest).unwrap(), b"new\n");
    }
}
