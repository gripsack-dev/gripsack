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

/// The run marker: which generation this journal's entries belong to.
/// Written before the first mutation; the flip makes it true.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RunMarker {
    /// The generation current pointed at when the run began (0026 §4):
    /// reconcile decides by EXACT equality — current == target is
    /// committed, current == previous is uncommitted, anything else is
    /// ambiguous and blocks. Numeric inequalities misclassify a
    /// crashed roll-FORWARD (current < target before the flip).
    /// None only in pre-0.23 markers, which reconcile by the 0.22
    /// direction rule.
    #[serde(default)]
    previous_generation: Option<u64>,
    target_generation: u64,
    /// 2 for markers written since 0.26 (0030 §11): distinguishes
    /// "no previous generation" (a fresh machine) from a pre-0.23
    /// marker that never recorded one — the latter refuses to
    /// reconcile, the former classifies exactly
    #[serde(default)]
    format: u8,
    /// Apply builds a NEWER generation (committed once `current`
    /// reaches the target); rollback returns to an OLDER one, so its
    /// commit condition inverts (committed once `current` comes back
    /// DOWN to the target) — 0025 §A. Default keeps pre-0.22 markers
    /// readable as apply runs.
    #[serde(default)]
    op: RunOp,
}

/// What the run is doing — the reconcile commit decision differs by
/// direction (see RunMarker).
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOp {
    /// The normal case: building the next generation.
    #[default]
    Apply,
    /// Returning to a previous generation.
    Rollback,
}

/// Declare the generation this run is building, BEFORE any mutation:
/// recovery compares it against `current` — a crash between the flip
/// and journal cleanup must NOT restore priors the committed
/// generation now owns (the post-commit window, review finding 5.1).
pub fn begin_run(
    home: &Dir,
    previous_generation: Option<u64>,
    target_generation: u64,
    op: RunOp,
) -> io::Result<()> {
    let marker = RunMarker {
        previous_generation,
        target_generation,
        op,
        format: 2,
    };
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
    let mut entry_paths = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        let rel = Path::new("journal").join(&name);
        // the marker is NOT deleted here — cleanup deletes it last
        if name != "run.json" && rel.extension().is_some_and(|e| e == "json") {
            entry_paths.push(rel);
        }
    }
    cleanup(home, &entry_paths)
}

/// Two durability barriers (0026 §5): entries deleted and fsync'd
/// FIRST, the marker deleted and fsync'd SECOND — so marker-durably-
/// gone implies entries-durably-gone. A single trailing fsync does
/// not order the deletions against a power loss; resurrected entries
/// with no marker read as an uncommitted run and would restore a
/// committed generation's priors (the 0.19.1 bug class, one level
/// down).
fn cleanup(home: &Dir, entry_paths: &[PathBuf]) -> io::Result<()> {
    for path in entry_paths {
        home.remove_file(path)?;
    }
    gripsack_fs::fsync_dir(home, Path::new("journal"))?;
    // a run that never mutated has no marker (end_run already
    // removed it) — absent is fine, anything else is real
    match home.remove_file(run_marker_rel()) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    gripsack_fs::fsync_dir(home, Path::new("journal"))
}

/// The run ended without mutating anything (satisfied, empty graph):
/// the marker declared by `begin_run` must not linger — a stale
/// marker with no entries is harmless but noisy, and a marker whose
/// target generation is later than `current` would misread the NEXT
/// crash window.
pub fn end_run(home: &Dir) -> io::Result<()> {
    // a stale marker misleads the NEXT crash window — its deletion is
    // a durability operation, never `let _ =` (0030 §12)
    match home.remove_file(run_marker_rel()) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    gripsack_fs::fsync_dir(home, Path::new("journal"))
}

/// What an uncommitted entry asks for, once resolved against the
/// current filesystem.
enum Recovery {
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
    // corruption or tampering, never a branch to guess). Pre-0.23
    // markers carry no previous generation; those reconcile by the
    // 0.22 direction rule, correct for those versions' semantics.
    let committed = match run_marker(home)? {
        Some(marker) => {
            // the ONE current-pointer reader (0030 §H10): recovery
            // never uses weaker commit evidence than normal commands
            let current = crate::generations::current(home_path)?;
            match classify(&RecoveryFacts {
                previous: marker.previous_generation,
                target: marker.target_generation,
                current,
                format: marker.format,
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
                Classification::Legacy => {
                    return Err(io::Error::other(format!(
                        "journal run marker (→{}) predates 0.23's exact \
                         transaction identity — cannot prove whether it \
                         committed. Inspect $GRIPSACK_HOME/journal: the \
                         entries name each destination's prior and intended \
                         state. To accept the current state, delete the \
                         journal directory; to restore, move the named files \
                         back by hand first",
                        marker.target_generation
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

/// Open a destination's parent as a capability, returning it with the
/// destination's bare file name. `open_or_create` because a crashed
/// run's destination may sit under parents the mutation itself was
/// about to create.
fn dest_capability(dest: &Path) -> io::Result<(Dir, PathBuf)> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let dir = gripsack_fs::open_or_create(parent)?;
    Ok((dir, PathBuf::from(dest.file_name().unwrap_or_default())))
}

fn run_marker(home: &Dir) -> io::Result<Option<RunMarker>> {
    match home.read(run_marker_rel()) {
        Ok(bytes) => serde_json::from_slice::<RunMarker>(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        // only NotFound means absent (same rule as capture): an
        // unreadable marker in RECOVERY code is commit evidence we
        // cannot see — error, never pick a branch blind (0025 §F)
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
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
fn decide_from(
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

/// The commit classification as a pure function (0028), exact
/// equality only: current == target is committed, current == previous
/// is uncommitted, anything else is ambiguous and blocks. Pre-0.23
/// markers carry no previous generation; those refuse (never
/// auto-classify by the model-proven-unsound direction rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Classification {
    Committed,
    Uncommitted,
    Ambiguous,
    /// A pre-0.23 run marker (no previous_generation): the commit
    /// state is unknowable - refuse, never auto-classify (0030 §11)
    Legacy,
}

/// The classifier's whole input, named: the facts a run marker
/// carries, plus the live `current` read at recovery time. (A struct,
/// not five positional `Option<u64>`-flavored arguments - the model
/// harness passes the same shape to its legacy counterexample.)
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecoveryFacts {
    /// The generation the run started from. None for a fresh
    /// machine's first run - and for pre-0.23 markers, which never
    /// recorded it (told apart by `format`).
    previous: Option<u64>,
    /// The generation the run was building toward.
    target: u64,
    /// `current` on disk when recovery ran.
    current: Option<u64>,
    /// Marker schema: 2+ carries exact transaction identity.
    format: u8,
}

fn classify(facts: &RecoveryFacts) -> Classification {
    match (facts.previous, facts.current) {
        (Some(_), Some(c)) if c == facts.target => Classification::Committed,
        (Some(prev), Some(c)) if c == prev => Classification::Uncommitted,
        (Some(_), _) => Classification::Ambiguous,
        // a fresh machine's run (no previous generation): exact too -
        // distinguishable from legacy by the format field
        (None, Some(c)) if facts.format >= 2 && c == facts.target => Classification::Committed,
        (None, None) if facts.format >= 2 => Classification::Uncommitted,
        (None, Some(_)) if facts.format >= 2 => Classification::Ambiguous,
        // pre-0.23 marker (no format field): the directional rule is
        // proven unsound (0028's kept counterexample) - refuse with
        // guidance rather than guess (0030 §11)
        _ => Classification::Legacy,
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
            match mode {
                Some(mode) => {
                    gripsack_fs::atomic_write_with_mode(dest_dir, dest_name, &bytes, *mode)?
                }
                // pre-0.24 entries carry no mode — content is still
                // restored; the mode is creation default
                None => gripsack_fs::atomic_write(dest_dir, dest_name, &bytes)?,
            }
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
