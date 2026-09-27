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
//! 1. **record** — before a mutation, capture the immediate prior and
//!    durably record it together with the intended post-state. Repeated
//!    mutations retain the run's FIRST prior and the latest immediate
//!    pre-state, so a crash before or after a later write restores the
//!    original object, not a partly updated hosting file.
//! 2. **mutate** — publish and sync the new object; the executor verifies
//!    the intended identity before another mutation may extend the chain.
//! 3. **commit_run** — after the generation flip, clean the journal.
//! 4. **reconcile** — restore uncommitted known intermediate/final states;
//!    preserve foreign edits. v1 entries retain their original three-way
//!    interpretation; v2 adds an explicit immediate pre-state.

pub mod marker;
pub(crate) mod recover;
mod storage;
mod wire;
pub use wire::{Entry, IntendedSerde, PriorSerde, WireRejection};

pub use marker::{RunOp, begin_run, commit_run, end_run};
pub use recover::{NoteSeverity, RecoveryNote, reconcile};

use gripsack_fs::Dir;
use std::io;
use std::path::{Path, PathBuf};
use storage::Journal;

/// The journal-domain identity of a live destination object (0031):
/// type + content identity from ONE observation. Files are
/// mode-aware; links compare their target verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectIdentity {
    File(crate::hash::FileIdentity),
    /// A symlink: its target. Compared byte-exact; adoption refuses
    /// non-UTF-8 targets before a link can be journaled.
    Link(String),
}

impl std::fmt::Display for ObjectIdentity {
    /// Human rendering for reports and error messages — never a
    /// comparison form (typed equality is the only comparison).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectIdentity::File(id) => write!(f, "file {id}"),
            ObjectIdentity::Link(target) => write!(f, "link → {target}"),
        }
    }
}

impl ObjectIdentity {
    /// Typed construction of the wire form (0045 F1).
    pub(crate) fn to_serde(&self) -> IntendedSerde {
        match self {
            ObjectIdentity::File(id) => IntendedSerde::File {
                identity: id.clone(),
            },
            ObjectIdentity::Link(target) => IntendedSerde::Link {
                target: target.clone(),
            },
        }
    }
}

impl From<&IntendedSerde> for ObjectIdentity {
    fn from(wire: &IntendedSerde) -> Self {
        match wire {
            IntendedSerde::File { identity } => ObjectIdentity::File(identity.clone()),
            IntendedSerde::Link { target } => ObjectIdentity::Link(target.clone()),
            // callers never convert the removal marker — see
            // Intended::from_wire
            IntendedSerde::Removed => unreachable!("Removed carries no object identity"),
        }
    }
}

/// The typed post-state a journaled mutation intends to create. Recovery
/// compares it with the run-original prior and, for v2, the immediate
/// pre-write state. A bytes-only hash cannot stand in for mode-aware identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intended {
    Removed,
    Object(ObjectIdentity),
}

impl Intended {
    /// Does this live identity satisfy the intent? Typed equality
    /// only (0045 F1): a link can never satisfy a file intent, no
    /// matter how its target spells.
    pub(crate) fn satisfied_by(&self, live: &ObjectIdentity) -> bool {
        matches!(self, Intended::Object(intended) if intended == live)
    }

    /// The object identity, when the intent is an object.
    pub fn as_object(&self) -> Option<&ObjectIdentity> {
        match self {
            Intended::Removed => None,
            Intended::Object(identity) => Some(identity),
        }
    }

    fn to_serde(&self) -> IntendedSerde {
        match self {
            Intended::Removed => IntendedSerde::Removed,
            Intended::Object(identity) => identity.to_serde(),
        }
    }

    /// Back to the typed intent (recovery's admission boundary:
    /// identities arrive decoded, never re-parsed from strings).
    pub(crate) fn from_wire(wire: &IntendedSerde) -> Intended {
        match wire {
            IntendedSerde::Removed => Intended::Removed,
            object => Intended::Object(ObjectIdentity::from(object)),
        }
    }
}

impl std::fmt::Display for Intended {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Intended::Removed => f.write_str("removed"),
            Intended::Object(identity) => identity.fmt(f),
        }
    }
}

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
    File {
        hash: crate::prior::PriorBlobId,
        mode: crate::prior::FileMode,
    },
    /// A symlink; recovery recreates it pointing at `target`.
    Symlink { target: String },
}

// The pre-0.40 `after` identity for a journaled REMOVAL was the bare
// string `"gripsack:removed"` — distinctive, but not disjoint from a
// symlink target (0045 F1). The tagged `IntendedSerde` replaced it;
// the literal survives only in `WireRejection::Legacy`'s detection
// of entries written before the tagged format.
/// Where the journal lives.
pub fn dir(home: &Path) -> PathBuf {
    home.join("journal")
}

/// Entry name relative to the pinned journal directory.
fn entry_name(dest: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.json",
        // the OS bytes, not a lossy spelling: two destinations may
        // never share an entry file because one was undecodable
        crate::hash::hex_sha256(dest.as_os_str().as_encoded_bytes())
    ))
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
        let hash = crate::prior::store_blob(home, &bytes)?;
        #[cfg(unix)]
        let mode = {
            use gripsack_fs::cap_std::fs::MetadataExt;
            meta.mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = 0o644;
        return Ok(Prior::File {
            hash,
            mode: crate::prior::FileMode::try_from(mode)?,
        });
    }
    // directories/fifos/devices are refused by deploy's guards; if
    // one is here anyway, treat it as absent — recovery will not
    // delete a directory through this path (remove_file only)
    Ok(Prior::Absent)
}

/// Record the intent to mutate `dest`: prior state AND the intended
/// post-mutation identity, durable BEFORE the mutation lands
pub fn record(home: &Dir, dest: &Path, prior: &Prior, after: &Intended) -> io::Result<()> {
    // 0045 F1: a lossy destination spelling would be restored to a
    // DIFFERENT path after a crash — refuse to journal it. The object
    // is untouched (record runs before the mutation) and the failed
    // run compensates. Destinations originate in the UTF-8 IR, so
    // this is defense in depth, not a user-facing rule.
    let dest_str = dest.to_str().ok_or_else(|| {
        io::Error::other(format!(
            "destination {} is not valid UTF-8 — refusing to journal it (the object is untouched)",
            dest.display()
        ))
    })?;
    let journal = Journal::prepare(home)?;
    let name = entry_name(dest);
    let before = PriorSerde::from(prior);
    let entry = match journal.read(&name) {
        Ok(bytes) => {
            let previous = Entry::from_wire(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            if previous.dest != dest_str {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "journal destination identity changed",
                ));
            }
            let observed = prior_identity(&before, home)?;
            let expected = Intended::from_wire(&previous.after);
            let follows_previous = match (&observed, &expected) {
                (None, Intended::Removed) => true,
                (Some(observed), Intended::Object(expected)) => observed == expected,
                _ => false,
            };
            if !follows_previous {
                return Err(io::Error::other(
                    "destination changed between journaled mutations; refusing to overwrite the intervening edit",
                ));
            }
            previous.advance(before, after.to_serde())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Entry::new(dest_str.to_owned(), before, after.to_serde())
        }
        Err(error) => return Err(error),
    };
    journal.write(
        &name,
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
fn read_uncommitted(journal: &Journal) -> io::Result<Vec<(PathBuf, Entry)>> {
    journal.restrict()?;
    let pending = journal.secure_quarantine()?;
    if pending != 0 {
        return Err(io::Error::other(format!(
            "{pending} quarantined journal entries remain; inspect $GRIPSACK_HOME/journal/quarantine before recovery"
        )));
    }
    let entries = journal.directory.read_dir(".")?;
    let mut out = Vec::new();
    let mut quarantined: Vec<String> = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        if name == storage::RUN_MARKER || name == "quarantine" {
            continue;
        }
        let rel = PathBuf::from(&name);
        // An unpublished atomic-write sibling is not recovery metadata.
        // Current siblings have a .tmp suffix; clean them durably on resume.
        if name.to_string_lossy().starts_with(".tmp-write-") {
            gripsack_fs::remove_file(&journal.directory, &rel)?;
            gripsack_fs::fsync_dir(&journal.directory, Path::new("."))?;
            continue;
        }
        if rel.extension().is_some_and(|e| e != "json") {
            continue;
        }
        // Admission (0045 F1): the entry decodes into validated types
        // here or not at all — legacy, unsupported-version and
        // malformed entries are quarantined with their reason, never
        // reinterpreted.
        // An IO failure is not evidence of malformed bytes. Retain the record
        // for another explicit recovery attempt instead of quarantining it.
        let bytes = journal.read(&rel).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "cannot read journal record {rel:?}; metadata retained for recovery: {error}"
                ),
            )
        })?;
        match Entry::from_wire(&bytes) {
            Ok(parsed) => out.push((rel, parsed)),
            Err(rejection) => {
                journal.quarantine(&rel)?;
                quarantined.push(format!("{}: {rejection}", name.to_string_lossy()));
            }
        }
    }
    if !quarantined.is_empty() {
        return Err(io::Error::other(format!(
            "{} journal entr{} could not be admitted — moved to journal/quarantine \
             under $GRIPSACK_HOME; inspect and remove them to continue \
             (recovery metadata is never ignored):\n  {}",
            quarantined.len(),
            if quarantined.len() == 1 { "y" } else { "ies" },
            quarantined.join("\n  "),
        )));
    }
    Ok(out)
}

/// Unfinished recovery state, for GC admission (0045 F3): a run
/// marker, journal entries, or a quarantined entry all mean recovery
/// has not run to completion and its evidence (prior blobs included)
/// is still load-bearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingRecovery {
    /// A declared run whose outcome was never classified.
    pub run_marker: bool,
    /// Uncommitted journal entries awaiting reconcile.
    pub entries: usize,
    /// Quarantined entries nobody has inspected yet.
    pub quarantined: usize,
}

impl std::fmt::Display for PendingRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if self.run_marker {
            parts.push("a run marker".to_string());
        }
        if self.entries > 0 {
            parts.push(format!(
                "{} uncommitted entr{}",
                self.entries,
                if self.entries == 1 { "y" } else { "ies" }
            ));
        }
        if self.quarantined > 0 {
            parts.push(format!(
                "{} quarantined entr{}",
                self.quarantined,
                if self.quarantined == 1 { "y" } else { "ies" }
            ));
        }
        f.write_str(&parts.join(", "))
    }
}

/// Is there recovery state a destructive command must not destroy
/// (0045 F3)? Read-only: no cleanup, no quarantine moves — admission
/// never mutates the evidence it inspects. An unreadable journal is
/// an error (fail closed), never "nothing pending".
pub fn pending_recovery(home: &Dir) -> io::Result<Option<PendingRecovery>> {
    let mut pending = PendingRecovery {
        run_marker: false,
        entries: 0,
        quarantined: 0,
    };
    let Some(journal) = Journal::open(home)? else {
        return Ok(None);
    };
    for entry in journal.directory.read_dir(".")? {
        let name = entry?.file_name();
        if name == storage::RUN_MARKER {
            pending.run_marker = true;
        } else if name == "quarantine" {
            if let Some(quarantine) = journal.quarantine_directory()? {
                for entry in quarantine.read_dir(".")? {
                    entry?;
                    pending.quarantined += 1;
                }
            }
        } else if Path::new(&name).extension().is_some_and(|e| e == "json")
            && !name.to_string_lossy().starts_with(".tmp-write-")
        {
            pending.entries += 1;
        }
    }
    Ok((pending.run_marker || pending.entries > 0 || pending.quarantined > 0).then_some(pending))
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
pub fn live_identity(dest_dir: &Dir, dest_name: &Path) -> io::Result<Option<ObjectIdentity>> {
    match dest_dir.symlink_metadata(dest_name) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target = dest_dir.read_link_contents(dest_name)?;
            // 0045 F1 (item 7): the same rule as capture, applied to
            // observation — a non-UTF-8 target lossily compared could
            // spell a journaled identity through replacement
            // characters. Refuse instead: the object is untouched,
            // the journal entry (if any) is retained.
            let Some(target) = target.to_str() else {
                return Err(io::Error::other(format!(
                    "{} is a symlink with a non-UTF-8 target — gripsack cannot \
                     establish its identity; move it aside to continue (it is \
                     preserved; nothing was deleted)",
                    dest_name.display()
                )));
            };
            Ok(Some(ObjectIdentity::Link(target.to_string())))
        }
        Ok(meta) => {
            // mode-aware (0031): the journal's file identity covers
            // the full permission set — a chmod between the drift
            // decision and the mutation aborts the run, and
            // chmod-only drift is never invisible
            use gripsack_fs::cap_std::fs::MetadataExt;
            let mode = meta.mode() & 0o7777;
            Ok(Some(ObjectIdentity::File(
                crate::hash::canonical_bytes_identity(&dest_dir.read(dest_name)?, mode),
            )))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The prior's identity in the same terms (recomputed from the blob;
/// the blob's own hash is the raw sha256 used for addressing).
fn prior_identity(prior: &PriorSerde, home: &Dir) -> io::Result<Option<ObjectIdentity>> {
    match prior {
        PriorSerde::Absent => Ok(None),
        PriorSerde::Symlink { target } => Ok(Some(ObjectIdentity::Link(target.clone()))),
        PriorSerde::File { hash, mode } => Ok(Some(ObjectIdentity::File(
            crate::hash::canonical_bytes_identity(
                &crate::prior::read_blob(home, hash)?,
                mode.bits(),
            ),
        ))),
    }
}

#[cfg(test)]
mod tests;

/// The exhaustive state-machine model of this protocol
/// (plan/0028) — see the module's own documentation.
#[cfg(test)]
mod model;
