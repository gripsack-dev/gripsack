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
pub use recover::{NoteSeverity, RecoveryNote, reconcile};

use gripsack_fs::Dir;
use std::io;
use std::path::{Path, PathBuf};

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

impl ObjectIdentity {
    /// The journal wire form (entries are one-version artifacts; the
    /// string round-trips through `Entry::after`). Recovery compares
    /// wire forms — canonical strings — never parses variants back.
    pub fn to_wire(&self) -> String {
        match self {
            ObjectIdentity::File(id) => id.to_string(),
            ObjectIdentity::Link(target) => target.clone(),
        }
    }
}

/// What a journaled mutation intends the destination to become —
/// the typed form of `Entry::after` (0026 §6's three-way decision:
/// live == intended → restore prior; live == prior → never landed;
/// else → someone's edit, keep it). Typed so a bytes-only template
/// hash can never be journaled where a mode-aware identity belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intended {
    Removed,
    Object(ObjectIdentity),
}

impl Intended {
    pub fn to_wire(&self) -> String {
        match self {
            Intended::Removed => REMOVED.to_string(),
            Intended::Object(id) => id.to_wire(),
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
    File { hash: String, mode: u32 },
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
        /// Original Unix mode — restored exactly (0026 §7).
        mode: u32,
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
    // prior blobs are user file bytes — secrets by nature (0033 R2):
    // they land 0600 and the directory is 0700, regardless of umask
    tighten_prior_dir(home)?;
    if present {
        // trust nothing by name (0029 §8): the blob exists — prove the
        // bytes or quarantine the impostor aside and write the truth
        let existing = home.read(&rel)?;
        if crate::hash::hex_sha256(&existing) != sha {
            let aside = Path::new("prior").join(format!("{sha}.corrupt"));
            gripsack_fs::rename(home, &rel, home, &aside)?;
            gripsack_fs::atomic_write_with_mode(home, &rel, bytes, 0o600)?;
        } else {
            // a pre-0.29 blob may sit at 0644 — tighten in place
            tighten_blob(home, &rel)?;
        }
    } else {
        gripsack_fs::atomic_write_with_mode(home, &rel, bytes, 0o600)?;
    }
    Ok(sha)
}

/// The prior directory is 0700 — blob names are content hashes, but
/// the listing itself leaks what files were ever adopted.
fn tighten_prior_dir(home: &Dir) -> io::Result<()> {
    home.create_dir_all(Path::new("prior"))?;
    #[cfg(unix)]
    {
        use gripsack_fs::cap_std::fs::MetadataExt as _;
        use std::os::unix::fs::PermissionsExt as _;
        let meta = home.symlink_metadata(Path::new("prior"))?;
        if meta.mode() & 0o777 != 0o700 {
            home.set_permissions(
                Path::new("prior"),
                gripsack_fs::cap_std::fs::Permissions::from_std(std::fs::Permissions::from_mode(
                    0o700,
                )),
            )?;
        }
    }
    Ok(())
}

/// A blob is 0600 — tighten a pre-0.29 one in place.
#[cfg(unix)]
fn tighten_blob(home: &Dir, rel: &Path) -> io::Result<()> {
    use gripsack_fs::cap_std::fs::MetadataExt as _;
    let meta = home.symlink_metadata(rel)?;
    if meta.mode() & 0o777 != 0o600 {
        use std::os::unix::fs::PermissionsExt as _;
        home.set_permissions(
            rel,
            gripsack_fs::cap_std::fs::Permissions::from_std(std::fs::Permissions::from_mode(0o600)),
        )?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn tighten_blob(_home: &Dir, _rel: &Path) -> io::Result<()> {
    Ok(())
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
            meta.mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = 0o644;
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
pub fn record(home: &Dir, dest: &Path, prior: &Prior, after: &Intended) -> io::Result<()> {
    let entry = Entry {
        dest: dest.to_string_lossy().into_owned(),
        prior: prior.into(),
        after: after.to_wire(),
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
        // An unpublished atomic-write sibling is not recovery metadata.
        // Current siblings have a .tmp suffix; clean them durably on resume.
        if name.to_string_lossy().starts_with(".tmp-write-") {
            gripsack_fs::remove_file(home, &rel)?;
            gripsack_fs::fsync_dir(home, Path::new("journal"))?;
            continue;
        }
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
                gripsack_fs::create_dir_all(home, &quarantine)?;
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
pub fn live_identity(dest_dir: &Dir, dest_name: &Path) -> io::Result<Option<ObjectIdentity>> {
    match dest_dir.symlink_metadata(dest_name) {
        Ok(meta) if meta.file_type().is_symlink() => Ok(Some(ObjectIdentity::Link(
            dest_dir
                .read_link_contents(dest_name)?
                .to_string_lossy()
                .into_owned(),
        ))),
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
            crate::hash::canonical_bytes_identity(&home.read(prior_blob_rel(hash))?, *mode),
        ))),
    }
}

#[cfg(test)]
mod tests;

/// The exhaustive state-machine model of this protocol
/// (plan/0028) — see the module's own documentation.
#[cfg(test)]
mod model;
