//! Journal recovery under crashes (plan/0056 F2 steps 1–2).
//!
//! The scenario runs below are the shipped journal run at storage-op level
//! (`begin_run` → leaf mutation → flip → `commit_run`); the kernel's apply
//! machine (F2 step 7) and rollback machine (F2 step 8) are also driven over
//! the same leaf protocol at the end of this file. Recovery is **not**
//! hand-written:
//! it is the kernel's recover machine driven by `gripsack_host::recover`
//! over `MemFs`, through a small test host that speaks this file's journal
//! wire. Every crash of the run — kill and every power-loss subset — is
//! followed by a recovery that is itself crashed everywhere, and then by a
//! recovery to completion; the oracle is checked on every image.
//!
//! The oracle (plan/0020's sentence; the cases of the deleted
//! `journal/model.rs` and `repeated_model.rs`): after recovery the journal
//! is empty, `current` names the previous or the target selection, and each
//! destination holds the content of the selected generation — or a
//! post-crash user edit, which is never overwritten. When `current` matches
//! neither selection, recovery refuses and changes nothing.
//!
//! Calibration: the 0.19.1 (no barrier) and 0.19.2 (trailing barrier only)
//! commit cleanups resurrect priors after a power loss, and a host that
//! commits by generation number (the pre-0028 direction heuristic) breaks
//! the oracle. S5: an external write the driver's re-check can see is never
//! overwritten; the window after it is measured, not assumed.

mod support;

use gripsack_host::journal_run::{
    Install, JournalRecord, Leaf, Observation, RunDriver, RunError, RunHost,
};
use gripsack_host::recover::{
    Found, HostEntry, LeafSite, RecoverError, RecoverHost, Recovery, RestoreSource, Settled,
    recover,
};
use gripsack_host::storage::{
    Boundary, Crash, CrashKinds, Exploration, ExploreError, ExternalWrite, MemDir, MemFs, Name,
    ObjectKind, Operation, PendingBound, Permissions, Storage, explore,
};
use gripsack_kernel::apply::{
    ApplyState, Command as ApplyCommand, CommitInputs, Event as ApplyEvent, OnDrift, Slot,
    UnitFacts as ApplyUnit, Want, apply_step,
};
use gripsack_kernel::journal_run::{Refusal as RunRefusal, Settle, UnitFacts};
use gripsack_kernel::ownership::Recorded;
use gripsack_kernel::ownership::{Consent, TakeOver};
use gripsack_kernel::recover::{
    Delegated, EntryFacts, Ident, Intended, Intended as KernelIntended, JournalFacts, Live,
    MarkerFacts, Outcome, Prior, Refusal, Route,
};
use gripsack_kernel::rollback::{
    Bytes, Command as RollbackCommand, Event as RollbackEvent, Place, PlanFacts, Profile,
    RollbackState, Target, UnitFacts as RollbackUnit, When, lands, rollback_step,
};
use gripsack_kernel::selection::{SelectionIdentity, TransactionId};
use gripsack_kernel::{DestId, Digest, GenerationId};
use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use support::{WriteOrder, atomic_write, name, symlink_replace};

const PREVIOUS: &str = "generations/1";
const TARGET: &str = "generations/2";
const RUN_MARKER: &str = "run.json";
const DEST_MODE: Permissions = Permissions::new(0o644);
const USER: &str = "user edit";
/// Every text a destination ever holds in these runs.
const TEXTS: &[&str] = &["one", "two", "mid", "old", "new", USER];

/// A destination's content in one generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    Absent,
    File(&'static str),
    Link(&'static str),
}

const ONE: Content = Content::File("one");
const TWO: Content = Content::File("two");
const MID: Content = Content::File("mid");
const EDITED: Content = Content::File(USER);

fn intern(text: &str) -> Option<&'static str> {
    TEXTS.iter().copied().find(|known| *known == text)
}

/// One destination the run changes.
#[derive(Debug, Clone, Copy)]
struct Dest {
    at: &'static str,
    prior: Content,
    intended: Content,
    /// Written twice: first `Some(first)`, then `intended` (journal v2's
    /// immediate pre-state).
    first: Option<Content>,
}

const fn dest(at: &'static str, prior: Content, intended: Content) -> Dest {
    Dest {
        at,
        prior,
        intended,
        first: None,
    }
}

/// `marker::cleanup`'s barrier placement in the run's `commit_run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cleanup {
    /// Shipped (0026 §5): unlink entries, fsync; unlink marker, fsync.
    TwoBarriers,
    /// 0.19.2: one trailing fsync — the entry barrier removed.
    TrailingBarrier,
    /// 0.19.1: no fsync at all.
    NoBarrier,
}

#[derive(Debug, Clone)]
struct Scenario {
    /// A fresh machine: no generation selected before the run.
    fresh: bool,
    dests: Vec<Dest>,
    cleanup: Cleanup,
}

impl Scenario {
    fn one(dest: Dest) -> Self {
        Self::many(vec![dest])
    }

    fn many(dests: Vec<Dest>) -> Self {
        Self {
            fresh: false,
            dests,
            cleanup: Cleanup::TwoBarriers,
        }
    }
}

// ---- the run (apply at storage-op level) ---------------------------------

fn put(fs: &MemFs, dir: &MemDir, at: &str, content: Content) -> io::Result<()> {
    match content {
        Content::File(text) => {
            atomic_write(fs, dir, at, text.as_bytes(), DEST_MODE, WriteOrder::Shipped)
        }
        Content::Link(target) => symlink_replace(fs, dir, at, target),
        Content::Absent => {
            match fs.unlink(dir, &name(at)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            fs.sync_dir(dir)
        }
    }
}

/// `$GRIPSACK_HOME` (where the destinations also live, for brevity) after
/// the previous generation deployed, all durable.
fn home(scenario: &Scenario) -> MemFs {
    let fs = MemFs::new();
    let root = fs.root().unwrap();
    for at in ["generations", "journal"] {
        fs.create_dir(&root, &name(at), Permissions::new(0o700))
            .unwrap();
    }
    let generations = fs.open_dir(&root, &name("generations")).unwrap();
    for at in ["1", "2", "3"] {
        fs.create_dir(&generations, &name(at), Permissions::new(0o755))
            .unwrap();
    }
    for dest in &scenario.dests {
        put(&fs, &root, dest.at, dest.prior).unwrap();
    }
    if !scenario.fresh {
        fs.symlink(&root, &name("current"), PREVIOUS.as_ref())
            .unwrap();
    }
    fs.settled()
}

fn journal(fs: &MemFs) -> io::Result<MemDir> {
    fs.open_dir(&fs.root()?, &name("journal"))
}

fn encode(content: Content) -> String {
    match content {
        Content::Absent => "-".into(),
        Content::File(text) => format!("F:{text}"),
        Content::Link(target) => format!("L:{target}"),
    }
}

fn decode(field: &str) -> io::Result<Content> {
    let torn = || io::Error::new(io::ErrorKind::InvalidData, "torn field");
    match field.split_at_checked(2) {
        _ if field == "-" => Ok(Content::Absent),
        Some(("F:", text)) => intern(text).map(Content::File).ok_or_else(torn),
        Some(("L:", target)) => intern(target).map(Content::Link).ok_or_else(torn),
        _ => Err(torn()),
    }
}

/// One entry on the wire: dest, prior, immediate pre-state, intended.
fn encode_entry(dest: &str, prior: Content, before: Content, intended: Content) -> Vec<u8> {
    format!(
        "{dest}\n{}\n{}\n{}",
        encode(prior),
        encode(before),
        encode(intended)
    )
    .into_bytes()
}

/// `marker::cleanup` with the barrier placement under test.
fn cleanup(fs: &MemFs, journal: &MemDir, entries: &[Name], order: Cleanup) -> io::Result<()> {
    for entry in entries {
        fs.unlink(journal, entry)?;
    }
    if order == Cleanup::TwoBarriers {
        fs.sync_dir(journal)?;
    }
    match fs.unlink(journal, &name(RUN_MARKER)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    if order != Cleanup::NoBarrier {
        fs.sync_dir(journal)?;
    }
    Ok(())
}

fn entry_names(fs: &MemFs, journal: &MemDir) -> io::Result<Vec<Name>> {
    Ok(fs
        .read_dir(journal)?
        .into_iter()
        .filter(|entry| {
            let text = entry.as_os_str().to_string_lossy();
            text != RUN_MARKER && !text.starts_with('.') && text.ends_with(".json")
        })
        .collect())
}

/// The run's side of this file's wire, for the kernel's journal-run machine.
struct RunWire<'a> {
    fs: &'a MemFs,
    /// The marker's selections, `previous\ntarget`.
    marker: String,
    /// The destination under way and what its entry records.
    unit: Option<(&'static str, Content, Content)>,
    records: Vec<Name>,
}

impl RunHost for RunWire<'_> {
    type Storage = MemFs;

    fn storage(&self) -> &MemFs {
        self.fs
    }

    fn journal_dir(&self) -> io::Result<MemDir> {
        journal(self.fs)
    }

    fn marker_name(&self) -> Name {
        name(RUN_MARKER)
    }

    fn marker_bytes(&self) -> io::Result<Vec<u8>> {
        Ok(self.marker.clone().into_bytes())
    }

    fn file_ident(&self, bytes: &[u8], mode: Permissions) -> Ident {
        Ident::File(digest(b'F', mode.bits(), bytes))
    }

    fn link_ident(&self, target: &Path) -> Option<Ident> {
        target
            .to_str()
            .map(|target| Ident::Link(digest(b'L', 0, target.as_bytes())))
    }

    fn staging_name(&self, at: &Name, link: bool) -> Name {
        let prefix = if link { ".tmp-link-" } else { ".tmp-write-" };
        name(&format!("{prefix}{}", at.as_os_str().to_string_lossy()))
    }

    /// The prior travels in the entry (this wire inlines content), so the
    /// entry is the prior's durable evidence.
    fn capture(&mut self, _: &UnitFacts, observed: &Observation) -> io::Result<JournalRecord> {
        let (at, prior, intended) = self.unit.take().expect("a unit is under way");
        let before = match observed {
            Observation::Absent => Content::Absent,
            Observation::File { bytes, .. } => {
                Content::File(intern(&String::from_utf8_lossy(bytes)).expect("a known text"))
            }
            Observation::Link { target } => {
                Content::Link(intern(&target.to_string_lossy()).expect("a known target"))
            }
            Observation::Unsupported(found) => panic!("captured {found}"),
        };
        Ok(JournalRecord {
            name: name(&format!("{at}.json")),
            bytes: encode_entry(at, prior, before, intended),
        })
    }

    fn settle(&mut self, _: Settle) -> io::Result<u32> {
        self.records = entry_names(self.fs, &journal(self.fs)?)?;
        Ok(self.records.len() as u32)
    }

    fn record_name(&self, index: u32) -> io::Result<Name> {
        Ok(self.records[index as usize].clone())
    }
}

impl<'a> RunWire<'a> {
    fn new(fs: &'a MemFs, previous: &str, target: &str) -> Self {
        Self {
            fs,
            marker: format!("{previous}\n{target}"),
            unit: None,
            records: Vec::new(),
        }
    }
}

fn live(content: Content) -> Live {
    ident(content).map_or(Live::Absent, Live::Obj)
}

fn intended(content: Content) -> KernelIntended {
    ident(content).map_or(KernelIntended::Removed, KernelIntended::Obj)
}

fn install(content: Content) -> Install<'static> {
    match content {
        Content::Absent => Install::Remove,
        Content::File(text) => Install::File {
            bytes: text.as_bytes().to_vec(),
            mode: DEST_MODE,
        },
        Content::Link(target) => Install::Link {
            target: PathBuf::from(target),
        },
    }
}

/// One destination write under the kernel's leaf protocol.
fn write(
    run: &mut RunDriver<MemFs>,
    host: &mut RunWire<'_>,
    dest: &Dest,
    from: Content,
    to: Content,
    follows: Option<Content>,
) -> io::Result<()> {
    host.unit = Some((dest.at, dest.prior, to));
    let leaf = Leaf {
        facts: UnitFacts {
            dest: DestId::new(0),
            expect: live(from),
            intended: intended(to),
            follows: follows.map(intended),
        },
        parent: host.fs.root()?,
        name: name(dest.at),
        install: install(to),
    };
    run.leaf(host, leaf).map(drop).map_err(RunError::into_io)
}

/// One committed apply through the kernel's journal-run machine: the marker,
/// each destination's leaf protocol, the flip, the cleanup.
fn apply(fs: &MemFs, scenario: &Scenario) -> io::Result<()> {
    let root = fs.root()?;
    let previous = if scenario.fresh { "none" } else { PREVIOUS };
    let mut host = RunWire::new(fs, previous, TARGET);
    let mut run = RunDriver::begin(&mut host).map_err(RunError::into_io)?;
    let first = |dest: &Dest| dest.first.unwrap_or(dest.intended);
    for dest in &scenario.dests {
        write(&mut run, &mut host, dest, dest.prior, first(dest), None)?;
    }
    // repeated writes: the entry's immediate pre-state is the first write
    for dest in &scenario.dests {
        if let Some(written) = dest.first {
            write(
                &mut run,
                &mut host,
                dest,
                written,
                dest.intended,
                Some(written),
            )?;
        }
    }
    // the flip: the commit point
    run.commit(&mut host).map_err(RunError::into_io)?;
    symlink_replace(fs, &root, "current", TARGET)?;
    match scenario.cleanup {
        Cleanup::TwoBarriers => run.flipped(&mut host).map_err(RunError::into_io),
        // the calibration mutants replace the kernel's cleanup
        mutant => {
            let journal = journal(fs)?;
            let entries = entry_names(fs, &journal)?;
            cleanup(fs, &journal, &entries, mutant)
        }
    }
}

// ---- the test host: this file's wire, for the kernel's recover machine ---

/// Exact identities for short test contents: kind, length, mode and bytes
/// laid into the digest (S7, collision resistance, is not under test here).
fn digest(tag: u8, mode: u32, bytes: &[u8]) -> Digest {
    assert!(bytes.len() <= 28, "test contents are short");
    let mut out = [0u8; 32];
    out[0] = tag;
    out[1] = bytes.len() as u8;
    out[2..4].copy_from_slice(&(mode as u16).to_le_bytes());
    out[4..4 + bytes.len()].copy_from_slice(bytes);
    Digest::from_bytes(out)
}

fn ident(content: Content) -> Option<Ident> {
    match content {
        Content::Absent => None,
        Content::File(text) => Some(Ident::File(digest(b'F', DEST_MODE.bits(), text.as_bytes()))),
        Content::Link(target) => Some(Ident::Link(digest(b'L', 0, target.as_bytes()))),
    }
}

fn selection(text: &str) -> io::Result<SelectionIdentity> {
    text.strip_prefix("generations/")
        .and_then(|number| number.parse().ok())
        .map(|number| SelectionIdentity::legacy(GenerationId::new(number)))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "torn selection"))
}

struct TestHost<'a> {
    fs: &'a MemFs,
    /// Mutant: commit whenever `current` is numerically at or before the
    /// target (absent counts as zero) — the pre-0028 direction rule.
    numeric_commit: bool,
    marker: Option<MarkerFacts>,
    entries: Vec<Name>,
}

impl<'a> TestHost<'a> {
    fn new(fs: &'a MemFs, numeric_commit: bool) -> Self {
        Self {
            fs,
            numeric_commit,
            marker: None,
            entries: Vec::new(),
        }
    }
}

impl RecoverHost for TestHost<'_> {
    type Storage = MemFs;

    fn storage(&self) -> &MemFs {
        self.fs
    }

    fn read_journal(&mut self) -> io::Result<Option<JournalFacts>> {
        let journal = match journal(self.fs) {
            Ok(journal) => journal,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        // `read_uncommitted`: unpublished temp siblings are not metadata
        for straggler in self.fs.read_dir(&journal)? {
            if straggler
                .as_os_str()
                .to_string_lossy()
                .starts_with(".tmp-write-")
            {
                self.fs.unlink(&journal, &straggler)?;
                self.fs.sync_dir(&journal)?;
            }
        }
        self.marker = match self.fs.read(&journal, &name(RUN_MARKER)) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let (previous, target) = text
                    .split_once('\n')
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "torn marker"))?;
                Some(MarkerFacts {
                    previous: match previous {
                        "none" => None,
                        previous => Some(selection(previous)?),
                    },
                    target: selection(target)?,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        self.entries = entry_names(self.fs, &journal)?;
        Ok(Some(JournalFacts {
            entries: self.entries.len() as u32,
            directories: 0,
            reverse_roots: 0,
            marker: self.marker,
        }))
    }

    fn read_current(&mut self) -> io::Result<Option<SelectionIdentity>> {
        let current = match self.fs.read_link(&self.fs.root()?, &name("current")) {
            Ok(target) => Some(selection(&target.to_string_lossy())?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if self.numeric_commit
            && let Some(marker) = self.marker
            && current.is_none_or(|current| current.generation() <= marker.target.generation())
        {
            return Ok(Some(marker.target));
        }
        Ok(current)
    }

    fn read_entry(&mut self, index: u32) -> io::Result<HostEntry<MemDir>> {
        let journal = journal(self.fs)?;
        let bytes = self.fs.read(&journal, &self.entries[index as usize])?;
        let text = String::from_utf8_lossy(&bytes);
        let fields: Vec<&str> = text.split('\n').collect();
        let [dest, prior, before, intended] = fields.as_slice() else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "torn entry"));
        };
        let (prior, before, intended) = (decode(prior)?, decode(before)?, decode(intended)?);
        let restore = match prior {
            Content::Absent => RestoreSource::Absent,
            Content::File(text) => RestoreSource::File {
                bytes: text.as_bytes().to_vec(),
                mode: DEST_MODE,
            },
            Content::Link(target) => RestoreSource::Link {
                target: PathBuf::from(target),
            },
        };
        Ok(HostEntry {
            facts: EntryFacts {
                dest: DestId::new(index),
                prior: ident(prior).map_or(Prior::Absent, Prior::Obj),
                before: if before == prior { None } else { ident(before) },
                intended: ident(intended).map_or(Intended::Removed, Intended::Obj),
                route: Route::Leaf,
            },
            site: Some(LeafSite {
                parent: self.fs.root()?,
                name: name(dest),
                restore,
            }),
        })
    }

    fn file_ident(&self, bytes: &[u8], mode: Permissions) -> Ident {
        Ident::File(digest(b'F', mode.bits(), bytes))
    }

    fn link_ident(&self, target: &Path) -> Option<Ident> {
        target
            .to_str()
            .map(|target| Ident::Link(digest(b'L', 0, target.as_bytes())))
    }

    fn staging_name(&self, at: &Name, link: bool) -> Name {
        let prefix = if link { ".tmp-link-" } else { ".tmp-write-" };
        name(&format!("{prefix}{}", at.as_os_str().to_string_lossy()))
    }

    fn delegate(&mut self, step: Delegated) -> io::Result<()> {
        Err(io::Error::other(format!(
            "no directory records in these runs, yet {step:?} was scheduled"
        )))
    }

    fn journal_dir(&self) -> io::Result<MemDir> {
        journal(self.fs)
    }

    fn record_name(&self, index: u32) -> io::Result<Name> {
        Ok(self.entries[index as usize].clone())
    }

    fn marker_name(&self) -> Name {
        name(RUN_MARKER)
    }

    fn selection_dir(&self) -> io::Result<MemDir> {
        self.fs.root()
    }
}

fn run_recovery(fs: &MemFs) -> Result<Recovery, RecoverError> {
    recover(&mut TestHost::new(fs, false))
}

// ---- the oracle -----------------------------------------------------------

fn content_at(fs: &MemFs, at: &str) -> Result<Content, String> {
    let root = fs.root().map_err(|e| e.to_string())?;
    let known = |text: String| intern(&text).ok_or(format!("{at} holds unknown {text:?}"));
    match fs.metadata(&root, &name(at)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Content::Absent),
        Err(error) => Err(error.to_string()),
        Ok(meta) => match meta.kind {
            ObjectKind::File if meta.permissions == DEST_MODE => {
                let bytes = fs.read(&root, &name(at)).map_err(|e| e.to_string())?;
                Ok(Content::File(known(
                    String::from_utf8_lossy(&bytes).into(),
                )?))
            }
            ObjectKind::Symlink => {
                let target = fs.read_link(&root, &name(at)).map_err(|e| e.to_string())?;
                Ok(Content::Link(known(target.to_string_lossy().into())?))
            }
            other => Err(format!("{at} is {other:?} at {}", meta.permissions)),
        },
    }
}

/// After recovery: the journal is empty, `current` names the previous or
/// the target selection, every destination holds that generation's
/// content — except the `edited` ones, which hold the user's edit.
fn recovered_state_is_consistent(
    fs: &MemFs,
    scenario: &Scenario,
    edited: &[&str],
) -> Result<(), String> {
    let root = fs.root().map_err(|e| e.to_string())?;
    let journal = journal(fs).map_err(|e| e.to_string())?;
    let left = fs.read_dir(&journal).map_err(|e| e.to_string())?;
    if !left.is_empty() {
        return Err(format!("journal not cleared: {left:?}"));
    }
    let committed = match fs.read_link(&root, &name("current")) {
        Ok(current) if current.as_os_str() == TARGET => true,
        Ok(current) if current.as_os_str() == PREVIOUS && !scenario.fresh => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound && scenario.fresh => false,
        other => return Err(format!("current is {other:?}")),
    };
    for dest in &scenario.dests {
        let found = content_at(fs, dest.at)?;
        let expected = if edited.contains(&dest.at) {
            EDITED
        } else if committed {
            dest.intended
        } else {
            dest.prior
        };
        if found != expected {
            if committed && found == dest.prior {
                return Err(format!(
                    "RESURRECTION: current selects the target but recovery restored \
                     {}'s prior {found:?}",
                    dest.at
                ));
            }
            return Err(format!(
                "{} holds {found:?}, expected {expected:?} (committed: {committed})",
                dest.at
            ));
        }
    }
    Ok(())
}

/// The user's editor replacing `at`: a complete, durable new file.
fn edit(fs: &MemFs, at: &str) -> Result<(), String> {
    let root = fs.root().map_err(|e| e.to_string())?;
    atomic_write(
        fs,
        &root,
        at,
        USER.as_bytes(),
        DEST_MODE,
        WriteOrder::Shipped,
    )
    .map_err(|e| e.to_string())
}

/// Crash the run everywhere; on every image optionally let the user edit
/// `edits.0`, then (if asked) crash the recovery everywhere too, optionally
/// let the user edit `edits.1`, and recover to completion.
fn explore_run(
    scenario: &Scenario,
    kinds: CrashKinds,
    crash_recovery_too: bool,
    edits: (Option<&'static str>, Option<&'static str>),
    numeric_commit: bool,
) -> Result<Exploration, ExploreError> {
    let nested = RefCell::new(Exploration::default());
    let recover_with = |fs: &MemFs| {
        recover(&mut TestHost::new(fs, numeric_commit))
            .map(drop)
            .map_err(RecoverError::into_io)
    };
    let finish = |fs: MemFs, edited: &[&str]| {
        recover_with(&fs).map_err(|e| format!("recovery failed: {e}"))?;
        recovered_state_is_consistent(&fs, scenario, edited)
    };
    let outer = explore(
        &home(scenario),
        kinds,
        PendingBound::default(),
        |fs| apply(fs, scenario),
        |image, _| {
            let mut edited: Vec<&str> = Vec::new();
            if let Some(at) = edits.0 {
                edit(&image, at)?;
                edited.push(at);
            }
            if !crash_recovery_too {
                return finish(image, &edited);
            }
            let inner = explore(
                &image,
                kinds,
                PendingBound::default(),
                recover_with,
                |again, _| {
                    let mut edited = edited.clone();
                    if let Some(at) = edits.1 {
                        edit(&again, at)?;
                        edited.push(at);
                    }
                    finish(again, &edited)
                },
            )
            .map_err(|error| format!("during recovery: {error}"))?;
            let mut nested = nested.borrow_mut();
            nested.cuts += inner.cuts;
            nested.images += inner.images;
            Ok(())
        },
    )?;
    let nested = nested.into_inner();
    Ok(Exploration {
        cuts: outer.cuts + nested.cuts,
        images: outer.images + nested.images,
    })
}

fn explored(scenario: &Scenario) -> Exploration {
    let explored = explore_run(scenario, CrashKinds::Both, true, (None, None), false)
        .unwrap_or_else(|error| panic!("{scenario:?}\n{error}"));
    eprintln!("{scenario:?}: {explored:?}");
    assert!(explored.images > explored.cuts);
    explored
}

// ---- every crash, every recovery crash ------------------------------------

#[test]
fn shipped_cleanup_survives_every_crash_and_every_recovery_crash() {
    // the step-1 exit, now with the kernel machine recovering
    let explored = explored(&Scenario::one(dest("dest", ONE, TWO)));
    assert!(explored.cuts > 20);
}

#[test]
fn every_entry_shape_recovers_on_fresh_and_existing_machines() {
    let shapes = [
        dest("dest", Content::Absent, TWO),      // create
        dest("dest", ONE, Content::Absent),      // remove (prune)
        dest("dest", Content::Link("old"), TWO), // link → file
        dest("dest", ONE, Content::Link("new")), // file → link
        dest("dest", Content::Link("old"), Content::Link("new")),
    ];
    for shape in shapes {
        for fresh in [false, true] {
            let mut scenario = Scenario::one(shape);
            scenario.fresh = fresh;
            explored(&scenario);
        }
    }
}

#[test]
fn repeated_writes_restore_the_run_original_prior() {
    for prior in [ONE, Content::Absent] {
        let mut repeated = dest("dest", prior, TWO);
        repeated.first = Some(MID);
        explored(&Scenario::one(repeated));
    }
}

#[test]
fn independent_destinations_recover() {
    explored(&Scenario::many(vec![
        dest("a", ONE, TWO),
        dest("b", Content::Absent, MID),
    ]));
}

// ---- post-crash edits, ambiguity, the commit mutant -----------------------

#[test]
fn edits_after_each_crash_are_never_overwritten() {
    let scenario = Scenario::many(vec![dest("a", ONE, TWO), dest("b", ONE, MID)]);
    // an edit after the run's crash, and another after the recovery's crash
    // (possibly after `b` was already restored)
    for edits in [(Some("a"), None), (None, Some("b")), (Some("a"), Some("b"))] {
        explore_run(&scenario, CrashKinds::Both, true, edits, false)
            .unwrap_or_else(|error| panic!("{edits:?}\n{error}"));
    }
}

/// The journal's published records (temp siblings excluded) with bytes.
fn journal_records(fs: &MemFs) -> Vec<(Name, Vec<u8>)> {
    let journal = journal(fs).unwrap();
    fs.read_dir(&journal)
        .unwrap()
        .into_iter()
        .filter(|at| !at.as_os_str().to_string_lossy().starts_with('.'))
        .map(|at| {
            let bytes = fs.read(&journal, &at).unwrap();
            (at, bytes)
        })
        .collect()
}

#[test]
fn ambiguous_current_retains_the_journal_and_touches_nothing() {
    let scenario = Scenario::many(vec![dest("a", ONE, TWO), dest("b", Content::Absent, MID)]);
    let contents = |fs: &MemFs| -> Vec<Content> {
        scenario
            .dests
            .iter()
            .map(|dest| content_at(fs, dest.at).unwrap())
            .collect()
    };
    let mut refused = 0;
    explore(
        &home(&scenario),
        CrashKinds::Kill,
        PendingBound::default(),
        |fs| apply(fs, &scenario),
        |image, point| {
            // a third selection, neither previous nor target
            let root = image.root().unwrap();
            symlink_replace(&image, &root, "current", "generations/3").unwrap();
            let records = journal_records(&image);
            let before = contents(&image);
            let marked = records.iter().any(|(at, _)| *at == name(RUN_MARKER));
            match run_recovery(&image) {
                Err(RecoverError::Refused {
                    refusal: Refusal::Ambiguous,
                    ..
                }) => refused += 1,
                Ok(_) if !marked => return Ok(()),
                other => return Err(format!("{point:?}: expected Ambiguous, got {other:?}")),
            }
            if journal_records(&image) != records || contents(&image) != before {
                return Err(format!("{point:?}: an ambiguous recovery changed state"));
            }
            Ok(())
        },
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(refused > 10, "the marker window was explored ({refused})");
}

#[test]
fn numeric_commit_mutant_breaks_the_oracle() {
    for fresh in [false, true] {
        let mut scenario = Scenario::one(dest("dest", ONE, TWO));
        scenario.fresh = fresh;
        let error = explore_run(&scenario, CrashKinds::Kill, false, (None, None), true)
            .expect_err("committing by generation number must be caught");
        let violation = error.violation().expect("an oracle violation");
        assert!(
            violation.failure.contains("holds"),
            "fresh={fresh}: {violation}"
        );
    }
}

// ---- the 0.19.x resurrections (step 1's calibration) ----------------------

#[test]
fn removing_the_entry_barrier_resurrects_priors_after_power_loss() {
    let mut scenario = Scenario::one(dest("dest", ONE, TWO));
    scenario.cleanup = Cleanup::TrailingBarrier;
    // A kill never finds it: the page cache keeps the unlinks in order.
    explore_run(&scenario, CrashKinds::Kill, true, (None, None), false)
        .unwrap_or_else(|error| panic!("kill alone must not resurrect: {error}"));

    let error = explore_run(&scenario, CrashKinds::PowerLoss, false, (None, None), false)
        .expect_err("MemFs must find the 0.19.2 resurrection");
    let violation = error.violation().expect("a resurrection, not a run error");
    eprintln!("{violation}");
    assert!(violation.failure.starts_with("RESURRECTION"));
    let Crash::PowerLoss { lost } = &violation.point.crash else {
        panic!("expected a power loss");
    };
    // the witness: the entry's unlink is lost, the marker's survives
    let unlinks: Vec<(usize, String)> = violation.trace[violation.first..]
        .iter()
        .enumerate()
        .filter(|(_, event)| event.boundary == Boundary::Unlink)
        .filter_map(|(i, event)| {
            let at = event.name.as_ref()?.as_os_str().to_string_lossy();
            (at == "dest.json" || at == RUN_MARKER).then(|| (violation.first + i, at.into_owned()))
        })
        .collect();
    let entry = unlinks.iter().find(|(_, at)| at == "dest.json").unwrap().0;
    let marker = unlinks.iter().find(|(_, at)| at == RUN_MARKER).unwrap().0;
    assert!(lost.contains(&entry), "entry unlink lost");
    assert!(!lost.contains(&marker), "marker unlink survived");
}

#[test]
fn the_0_19_1_cleanup_without_barriers_resurrects_after_returning() {
    let mut scenario = Scenario::one(dest("dest", ONE, TWO));
    scenario.cleanup = Cleanup::NoBarrier;
    let error = explore_run(&scenario, CrashKinds::PowerLoss, false, (None, None), false)
        .expect_err("MemFs must find the 0.19.1 resurrection");
    let violation = error.violation().expect("a resurrection");
    assert!(violation.failure.starts_with("RESURRECTION"), "{violation}");
}

// ---- the unrestorable-entry wedge (0058 item 1) ---------------------------

/// The run crashed after creating `dest` (prior absent); the user then put a
/// directory there. Recovery refuses by name and changes nothing; once the
/// directory is moved aside, recovery completes.
#[test]
fn a_directory_where_a_file_is_restored_is_refused_by_name_until_moved() {
    let scenario = Scenario::one(dest("dest", Content::Absent, TWO));
    let mut wedged = 0;
    explore(
        &home(&scenario),
        CrashKinds::Kill,
        PendingBound::default(),
        |fs| apply(fs, &scenario),
        |image, point| {
            if content_at(&image, "dest") != Ok(TWO) {
                return Ok(());
            }
            let root = image.root().unwrap();
            let journal_dir = journal(&image).unwrap();
            let recorded = image.read(&journal_dir, &name("dest.json")).is_ok();
            let committed = image
                .read_link(&root, &name("current"))
                .is_ok_and(|current| current.as_os_str() == TARGET);
            image.unlink(&root, &name("dest")).unwrap();
            image
                .create_dir(&root, &name("dest"), Permissions::new(0o755))
                .unwrap();
            let user_dir = image.open_dir(&root, &name("dest")).unwrap();
            atomic_write(
                &image,
                &user_dir,
                "keep",
                USER.as_bytes(),
                DEST_MODE,
                WriteOrder::Shipped,
            )
            .unwrap();
            match run_recovery(&image) {
                Err(RecoverError::Refused {
                    refusal: Refusal::Unrestorable { entry: 0, .. },
                    found: Some(Found::Directory),
                }) if recorded && !committed => wedged += 1,
                Ok(_) if !recorded || committed => return Ok(()),
                other => return Err(format!("{point:?}: {other:?}")),
            }
            // fail-closed: the entry and the user's directory are intact
            if image.read(&journal_dir, &name("dest.json")).is_err()
                || image.read(&user_dir, &name("keep")).ok() != Some(USER.as_bytes().to_vec())
            {
                return Err(format!("{point:?}: the refusal changed state"));
            }
            // the repair: move it aside, recover again
            image
                .rename(&root, &name("dest"), &root, &name("dest.aside"))
                .unwrap();
            let recovery = run_recovery(&image).map_err(|e| format!("after repair: {e}"))?;
            if !matches!(recovery.outcome, Outcome::Recovered) {
                return Err(format!("after repair: {:?}", recovery.outcome));
            }
            recovered_state_is_consistent(&image, &scenario, &[])?;
            let aside = image.open_dir(&root, &name("dest.aside")).unwrap();
            if image.read(&aside, &name("keep")).ok() != Some(USER.as_bytes().to_vec()) {
                return Err("the moved directory lost the user's file".into());
            }
            Ok(())
        },
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(wedged > 0, "the wedge window was explored");
}

// ---- S5: external writers -------------------------------------------------

/// A crashed run left `dest` holding the run's write; recovery restores the
/// prior. An external write of `dest` is landed before each storage call of
/// that recovery in turn. Whenever the driver's re-check (its last lookup of
/// `dest` by name before the rename that restores it) can see the write, the
/// user's bytes survive — kept as a foreign edit when the decision saw them,
/// or left alone by a drift refusal when only the re-check did. The only
/// operations at which they are overwritten lie between that lookup and the
/// rename: the named residual window.
#[test]
fn recovery_never_overwrites_an_external_write_its_recheck_sees() {
    for prior in [ONE, Content::Link("old")] {
        let scenario = Scenario::one(dest("dest", prior, TWO));
        let crashed = home(&scenario);
        let total = {
            let probe = crashed.snapshot();
            apply(&probe, &scenario).unwrap();
            probe.effects()
        };
        // the run, killed after its mutation of dest landed, before the flip
        let image = (0..total)
            .map(|cut| {
                let fs = crashed.snapshot();
                fs.crash_at(cut);
                let _ = apply(&fs, &scenario);
                fs.kill()
            })
            .find(|fs| {
                content_at(fs, "dest") == Ok(TWO)
                    && fs.read_link(&fs.root().unwrap(), &name("current")).ok()
                        == Some(PREVIOUS.into())
            })
            .expect("a crash after the mutation, before the flip");
        // obtained once, so later snapshots number their operations alike
        let root = image.root().unwrap();

        // locate the act and the re-check in an undisturbed recovery
        let probe = image.snapshot();
        let first = probe.operations().len();
        run_recovery(&probe).unwrap();
        let operations = probe.operations();
        let named_dest = |op: &Operation| op.name.as_ref() == Some(&name("dest"));
        let act = (first..operations.len())
            .find(|&i| {
                operations[i].boundary == Some(Boundary::FilePublish) && named_dest(&operations[i])
            })
            .expect("the restoring rename");
        let recheck = (first..act)
            .rev()
            .find(|&i| operations[i].boundary.is_none() && named_dest(&operations[i]))
            .expect("the re-check lookup");

        let mut overwritten = Vec::new();
        for at in first..=operations.len() {
            let fs = image.snapshot();
            fs.inject(
                at,
                ExternalWrite {
                    dir: root,
                    name: name("dest"),
                    bytes: USER.as_bytes().to_vec(),
                    permissions: DEST_MODE,
                },
            );
            let result = run_recovery(&fs);
            if content_at(&fs, "dest") != Ok(EDITED) {
                overwritten.push(at);
                assert!(
                    at > recheck && at <= act,
                    "{prior:?}: the write before operation {at} was visible to the \
                     re-check (lookup {recheck}, act {act}) and was overwritten: {result:?}"
                );
            } else if at <= recheck {
                match &result {
                    Ok(recovery) => assert!(
                        recovery
                            .notes
                            .iter()
                            .any(|note| note.settled == Settled::Kept),
                        "{prior:?}: at {at} the edit survived unreported"
                    ),
                    Err(RecoverError::Refused {
                        refusal: Refusal::Drifted { .. },
                        ..
                    }) => {}
                    Err(other) => panic!("{prior:?}: at {at}: {other}"),
                }
            }
        }
        // the window exists, and only between the re-check and the act
        assert!(!overwritten.is_empty(), "{prior:?}: the S5 window exists");
        eprintln!(
            "{prior:?}: S5 window of {} operation(s) between re-check {recheck} and act {act}",
            overwritten.len()
        );
    }
}

// ---- S5 in the leaf protocol: the re-check before the act ----------------

/// The 0.53 leaf order, restated for comparison: the destination is checked
/// against the plan once, at capture; the entry is published; the write
/// follows with no re-check. (The marker, flip and cleanup are the kernel's
/// order; only the leaf differs.)
fn apply_without_recheck(fs: &MemFs, dest: &Dest) -> io::Result<()> {
    let root = fs.root()?;
    let journal = journal(fs)?;
    atomic_write(
        fs,
        &journal,
        RUN_MARKER,
        format!("{PREVIOUS}\n{TARGET}").as_bytes(),
        Permissions::new(0o600),
        WriteOrder::Shipped,
    )?;
    // capture: one observation, compared with the plan's
    if content_at(fs, dest.at).map_err(io::Error::other)? != dest.prior {
        return Err(io::Error::other(
            "changed between the drift decision and the mutation",
        ));
    }
    atomic_write(
        fs,
        &journal,
        &format!("{}.json", dest.at),
        &encode_entry(dest.at, dest.prior, dest.prior, dest.intended),
        Permissions::new(0o600),
        WriteOrder::Shipped,
    )?;
    put(fs, &root, dest.at, dest.intended)
}

/// The operations at which an external write of `dest`, landed just before
/// them, is overwritten by the run (the S5 window), and the run's operations.
fn overwritten_by(
    image: &MemFs,
    run: impl Fn(&MemFs) -> io::Result<()>,
) -> (Vec<usize>, Vec<Operation>) {
    let root = image.root().unwrap();
    let probe = image.snapshot();
    let first = probe.operations().len();
    run(&probe).unwrap();
    let operations = probe.operations();
    let mut overwritten = Vec::new();
    for at in first..=operations.len() {
        let fs = image.snapshot();
        fs.inject(
            at,
            ExternalWrite {
                dir: root,
                name: name("dest"),
                bytes: USER.as_bytes().to_vec(),
                permissions: DEST_MODE,
            },
        );
        let _ = run(&fs);
        if content_at(&fs, "dest") != Ok(EDITED) {
            overwritten.push(at);
        }
    }
    (overwritten, operations[first..].to_vec())
}

/// An external write of the destination landing anywhere in the run is kept
/// unless it lands between the act's re-check (its last lookup of `dest` by
/// name) and the rename — two operations — and the run then stops on a
/// drift refusal rather than overwrite it. Measured against the 0.53 leaf
/// order, whose window also spans the entry's publication.
#[test]
fn the_leaf_never_overwrites_an_external_write_its_recheck_sees() {
    let dest = dest("dest", ONE, TWO);
    let scenario = Scenario::one(dest);
    let image = home(&scenario);
    // obtained once, so later snapshots number their operations alike
    let _ = image.root().unwrap();

    let (overwritten, operations) = overwritten_by(&image, |fs| apply(fs, &scenario));
    let first = image.snapshot().operations().len();
    let named_dest = |op: &Operation| op.name.as_ref() == Some(&name("dest"));
    let act = (0..operations.len())
        .find(|&i| {
            operations[i].boundary == Some(Boundary::FilePublish) && named_dest(&operations[i])
        })
        .expect("the installing rename")
        + first;
    let recheck = (0..act - first)
        .rev()
        .find(|&i| operations[i].boundary.is_none() && named_dest(&operations[i]))
        .expect("the re-check lookup")
        + first;
    assert!(
        overwritten.iter().all(|&at| at > recheck && at <= act),
        "kernel leaf: overwritten at {overwritten:?}, re-check {recheck}, act {act}"
    );
    assert!(!overwritten.is_empty(), "the S5 window exists");

    let (baseline, _) = overwritten_by(&image, |fs| apply_without_recheck(fs, &dest));
    eprintln!(
        "S5 window: kernel leaf {} operation(s) {overwritten:?}; 0.53 leaf {} operation(s) {baseline:?}",
        overwritten.len(),
        baseline.len()
    );
    // the 0.53 order overwrote a write landing while its entry was published
    assert!(baseline.len() > overwritten.len() + 4, "{baseline:?}");
}

// ---- a run that journals nothing ------------------------------------------

/// `end_run`: the marker is published, then removed under the kernel's
/// cleanup order. Every crash recovers to an empty journal and the previous
/// selection.
#[test]
fn an_ended_run_leaves_nothing_after_any_crash() {
    let scenario = Scenario::many(Vec::new());
    let explored = explore(
        &home(&scenario),
        CrashKinds::Both,
        PendingBound::default(),
        |fs| {
            let mut host = RunWire::new(fs, PREVIOUS, TARGET);
            let mut run = RunDriver::begin(&mut host).map_err(RunError::into_io)?;
            run.end(&mut host).map_err(RunError::into_io)
        },
        |image, _| {
            run_recovery(&image).map_err(|e| format!("recovery failed: {e}"))?;
            recovered_state_is_consistent(&image, &scenario, &[])?;
            match image.read_link(&image.root().unwrap(), &name("current")) {
                Ok(current) if current.as_os_str() == PREVIOUS => Ok(()),
                other => Err(format!("current is {other:?}")),
            }
        },
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(explored.cuts >= 8, "{explored:?}");
}

/// The kernel refuses to end a run that journaled a destination: its marker
/// and entry stay for recovery.
#[test]
fn a_run_that_journaled_a_destination_cannot_end() {
    let scenario = Scenario::one(dest("dest", ONE, TWO));
    let fs = home(&scenario);
    let mut host = RunWire::new(&fs, PREVIOUS, TARGET);
    let mut run = RunDriver::begin(&mut host).unwrap();
    write(&mut run, &mut host, &scenario.dests[0], ONE, TWO, None).unwrap();
    let error = run.end(&mut host).unwrap_err();
    assert!(
        matches!(error.refusal(), Some(RunRefusal::Unfinished)),
        "{error}"
    );
    let records: Vec<Name> = journal_records(&fs).into_iter().map(|(at, _)| at).collect();
    assert_eq!(records, vec![name("dest.json"), name(RUN_MARKER)]);
}

// ---- rollback over the leaf protocol (§16.7 deviation 1) ------------------

const CURRENT_GEN: &str = "generations/2";
const ROLLBACK_TO: &str = "generations/1";

/// One unit of a rollback: what generation 2 (current) deployed, what
/// generation 1 (the target) recorded.
#[derive(Debug, Clone, Copy)]
struct Unit {
    at: &'static str,
    current: Content,
    target: Content,
}

fn rollback_home(units: &[Unit]) -> MemFs {
    let fs = MemFs::new();
    let root = fs.root().unwrap();
    for at in ["generations", "journal"] {
        fs.create_dir(&root, &name(at), Permissions::new(0o700))
            .unwrap();
    }
    let generations = fs.open_dir(&root, &name("generations")).unwrap();
    for at in ["1", "2"] {
        fs.create_dir(&generations, &name(at), Permissions::new(0o755))
            .unwrap();
    }
    for unit in units {
        put(&fs, &root, unit.at, unit.current).unwrap();
    }
    fs.symlink(&root, &name("current"), CURRENT_GEN.as_ref())
        .unwrap();
    fs.settled()
}

fn transaction(generation: u64, byte: u8) -> SelectionIdentity {
    SelectionIdentity::transaction(
        GenerationId::new(generation),
        TransactionId::from_bytes([byte; 32]),
    )
}

/// The product's rollback at storage-op level: the kernel rollback machine
/// decides every unit, each write goes through the journal run's leaf
/// protocol, the flip reselects generation 1, the run cleans up.
fn rollback(fs: &MemFs, units: &[Unit]) -> io::Result<()> {
    let root = fs.root()?;
    let mut host = RunWire::new(fs, CURRENT_GEN, ROLLBACK_TO);
    let mut run = RunDriver::begin(&mut host).map_err(RunError::into_io)?;
    let mut state = RollbackState::new();
    let mut event = RollbackEvent::Start;
    loop {
        let (next, command) = rollback_step(state, event);
        state = next;
        event = match command {
            RollbackCommand::ReadPlan => RollbackEvent::PlanRead(PlanFacts {
                units: units.len() as u32,
                reselect: transaction(1, 2),
                previous: Some(transaction(2, 1)),
                profile: Profile::Retained,
            }),
            RollbackCommand::ReadUnit(k) => {
                let unit = units[k as usize];
                RollbackEvent::UnitRead(RollbackUnit {
                    dest: DestId::new(k),
                    place: Place::Outside,
                    route: Route::Leaf,
                    current: ident(unit.current).map_or(Recorded::Unrecorded, Recorded::Wrote),
                    target: match ident(unit.target) {
                        Some(ident) => Target::Wrote {
                            ident,
                            bytes: Bytes::Retained,
                        },
                        None => Target::Released(KernelIntended::Removed),
                    },
                    when: When::AfterShape,
                })
            }
            RollbackCommand::Observe { unit, dest } => RollbackEvent::Observed {
                dest,
                live: live(content_at(fs, units[unit as usize].at).map_err(io::Error::other)?),
            },
            RollbackCommand::Mutate {
                unit,
                expect,
                install: wanted,
                ..
            } => {
                let Unit {
                    at,
                    current,
                    target: to,
                } = units[unit as usize];
                assert!(lands(&wanted, &live(to)), "the machine installs the target");
                host.unit = Some((at, current, to));
                let leaf = Leaf {
                    facts: UnitFacts {
                        dest: DestId::new(unit),
                        expect,
                        intended: wanted,
                        follows: None,
                    },
                    parent: root,
                    name: name(at),
                    install: install(to),
                };
                match run.leaf(&mut host, leaf) {
                    Ok(landed) => {
                        // the driver hands back what the leaf protocol saw land
                        let observed = live(content_at(fs, at).map_err(io::Error::other)?);
                        assert!(landed == observed, "the leaf reports what landed");
                        RollbackEvent::Landed { live: landed }
                    }
                    Err(error) => return Err(error.into_io()),
                }
            }
            RollbackCommand::Directory { .. } => {
                return Err(io::Error::other("no directory items here"));
            }
            RollbackCommand::Report { .. } => RollbackEvent::Done,
            RollbackCommand::Delegate { .. } => {
                return Err(io::Error::other("no delegated units here"));
            }
            RollbackCommand::Flip(_) => {
                run.commit(&mut host).map_err(RunError::into_io)?;
                symlink_replace(fs, &root, "current", ROLLBACK_TO)?;
                RollbackEvent::Done
            }
            RollbackCommand::Finish(_) => {
                return run.flipped(&mut host).map_err(RunError::into_io);
            }
            RollbackCommand::Abort(refusal) => {
                return Err(io::Error::other(format!("rollback refused: {refusal:?}")));
            }
        };
    }
}

/// After recovery: the journal is empty and either the rollback committed
/// (`current` is generation 1, every destination holds its record there) or
/// it did not (generation 2, every destination holds what it deployed).
fn rolled_back_consistently(fs: &MemFs, units: &[Unit]) -> Result<(), String> {
    let root = fs.root().map_err(|e| e.to_string())?;
    let left = fs
        .read_dir(&journal(fs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if !left.is_empty() {
        return Err(format!("journal not cleared: {left:?}"));
    }
    let committed = match fs.read_link(&root, &name("current")) {
        Ok(current) if current.as_os_str() == ROLLBACK_TO => true,
        Ok(current) if current.as_os_str() == CURRENT_GEN => false,
        other => return Err(format!("current is {other:?}")),
    };
    for unit in units {
        let found = content_at(fs, unit.at)?;
        let expected = if committed { unit.target } else { unit.current };
        if found != expected {
            return Err(format!(
                "{} holds {found:?}, expected {expected:?} (rolled back: {committed})",
                unit.at
            ));
        }
    }
    Ok(())
}

/// The rollback machine's writes go through the leaf protocol over MemFs:
/// every kill and power-loss cut of the rollback, every cut of the recovery
/// after it, then recovery to the end, leaves either the generation rolled
/// back from or the one rolled back to — exactly, per destination.
#[test]
fn rollback_survives_every_crash_and_every_recovery_crash() {
    let units = [
        Unit {
            at: "a",
            current: TWO,
            target: ONE,
        },
        Unit {
            at: "b",
            current: MID,
            target: Content::Absent,
        },
        Unit {
            at: "c",
            current: Content::Link("new"),
            target: Content::Link("old"),
        },
    ];
    let nested = RefCell::new(Exploration::default());
    let recover_with = |fs: &MemFs| run_recovery(fs).map(drop).map_err(RecoverError::into_io);
    let outer = explore(
        &rollback_home(&units),
        CrashKinds::Both,
        PendingBound::default(),
        |fs| rollback(fs, &units),
        |image, _| {
            let inner = explore(
                &image,
                CrashKinds::Both,
                PendingBound::default(),
                recover_with,
                |again, _| {
                    recover_with(&again).map_err(|e| format!("recovery failed: {e}"))?;
                    rolled_back_consistently(&again, &units)
                },
            )
            .map_err(|error| format!("during recovery: {error}"))?;
            let mut nested = nested.borrow_mut();
            nested.cuts += inner.cuts;
            nested.images += inner.images;
            Ok(())
        },
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let nested = nested.into_inner();
    eprintln!("rollback: {outer:?}, recoveries: {nested:?}");
    assert!(outer.cuts > 30 && nested.images > outer.images);
}

// ---- apply over the leaf protocol (plan/0056 F2 step 7) -------------------

const APPLY_FROM: &str = "generations/1";
const APPLY_TO: &str = "generations/2";

/// One unit of an apply: what the destination holds (gripsack's last write
/// there, or nothing), and what the run wants: a deploy, or a release that
/// brings back nothing.
#[derive(Debug, Clone, Copy)]
struct Applied {
    at: &'static str,
    held: Content,
    /// `Some` deploys this content; `None` releases the destination.
    deploy: Option<Content>,
}

/// The product's apply at storage-op level: the kernel apply machine decides
/// every unit (a release of a destination a deploy claims is never written),
/// each write goes through the journal run's leaf protocol, the machine
/// admits the publication and the flip, and the run cleans up.
fn apply_run(fs: &MemFs, units: &[Applied]) -> io::Result<()> {
    let root = fs.root()?;
    let mut host = RunWire::new(fs, APPLY_FROM, APPLY_TO);
    let mut run = RunDriver::begin(&mut host).map_err(RunError::into_io)?;
    let mut state = ApplyState::new();
    let mut event = ApplyEvent::Start;
    let dest_of =
        |at: &str| DestId::new(units.iter().position(|unit| unit.at == at).unwrap_or(0) as u32);
    loop {
        let (next, command) = apply_step(state, event);
        state = next;
        event = match command {
            ApplyCommand::ReadUnit(k) => match units.get(k as usize) {
                None => ApplyEvent::Exhausted,
                Some(unit) => ApplyEvent::UnitRead(ApplyUnit {
                    dest: dest_of(unit.at),
                    slot: Slot::Whole,
                    route: Route::Leaf,
                    recorded: ident(unit.held).map_or(Recorded::Unrecorded, Recorded::Wrote),
                    want: match unit.deploy {
                        Some(content) => {
                            let ident = ident(content).expect("a deploy wants an object");
                            Want::Deploy {
                                ident,
                                adopted: ident,
                            }
                        }
                        None => Want::Release(KernelIntended::Removed),
                    },
                    consent: Consent {
                        take_over: TakeOver::Withheld,
                    },
                    drift: OnDrift::Keep,
                }),
            },
            ApplyCommand::Observe { unit, dest } => ApplyEvent::Observed {
                dest,
                live: live(content_at(fs, units[unit as usize].at).map_err(io::Error::other)?),
            },
            ApplyCommand::Mutate {
                unit,
                expect,
                install: wanted,
                ..
            } => {
                let Applied { at, held, deploy } = units[unit as usize];
                let to = deploy.unwrap_or(Content::Absent);
                assert!(lands(&wanted, &live(to)), "the machine installs the want");
                host.unit = Some((at, held, to));
                let leaf = Leaf {
                    facts: UnitFacts {
                        dest: DestId::new(unit),
                        expect,
                        intended: wanted,
                        follows: None,
                    },
                    parent: root,
                    name: name(at),
                    install: install(to),
                };
                match run.leaf(&mut host, leaf) {
                    Ok(landed) => ApplyEvent::Landed { live: landed },
                    Err(error) => return Err(error.into_io()),
                }
            }
            ApplyCommand::Report { .. } => ApplyEvent::Done,
            ApplyCommand::Delegate { .. } => {
                return Err(io::Error::other("no delegated units here"));
            }
            ApplyCommand::Directory { .. } => {
                return Err(io::Error::other("no directory items here"));
            }
            ApplyCommand::ReadCommit => ApplyEvent::CommitRead(CommitInputs {
                previous: Some(SelectionIdentity::legacy(GenerationId::new(1))),
                reserved: transaction(2, 2),
                allocated: GenerationId::new(2),
                floor: Some(GenerationId::new(1)),
                unchanged: false,
                exists: false,
            }),
            // the generation directory's publication is the store's own
            // protocol, not modelled here: generation 2 already exists
            ApplyCommand::Publish(_) => ApplyEvent::Done,
            ApplyCommand::Flip(_) => {
                run.commit(&mut host).map_err(RunError::into_io)?;
                symlink_replace(fs, &root, "current", APPLY_TO)?;
                run.flipped(&mut host).map_err(RunError::into_io)?;
                ApplyEvent::Done
            }
            ApplyCommand::Activate => ApplyEvent::Done,
            ApplyCommand::Finish(_) => return Ok(()),
            ApplyCommand::Abort(refusal) => {
                return Err(io::Error::other(format!("apply refused: {refusal:?}")));
            }
        };
    }
}

fn apply_home(units: &[Applied]) -> MemFs {
    let fs = rollback_home(&[]);
    let root = fs.root().unwrap();
    let mut seen: Vec<&str> = Vec::new();
    for unit in units {
        if !seen.contains(&unit.at) {
            seen.push(unit.at);
            put(&fs, &root, unit.at, unit.held).unwrap();
        }
    }
    fs.unlink(&root, &name("current")).unwrap();
    fs.symlink(&root, &name("current"), APPLY_FROM.as_ref())
        .unwrap();
    fs.settled()
}

/// After recovery: the journal is empty and either the apply committed
/// (`current` is generation 2, every destination holds what the run wanted
/// — a released one nothing, unless a deploy claims it) or it did not
/// (generation 1, every destination holds what it held).
fn applied_consistently(fs: &MemFs, units: &[Applied]) -> Result<(), String> {
    let root = fs.root().map_err(|e| e.to_string())?;
    let left = fs
        .read_dir(&journal(fs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if !left.is_empty() {
        return Err(format!("journal not cleared: {left:?}"));
    }
    let committed = match fs.read_link(&root, &name("current")) {
        Ok(current) if current.as_os_str() == APPLY_TO => true,
        Ok(current) if current.as_os_str() == APPLY_FROM => false,
        other => return Err(format!("current is {other:?}")),
    };
    for unit in units {
        let deployed = units
            .iter()
            .find_map(|other| (other.at == unit.at).then_some(other.deploy).flatten());
        let found = content_at(fs, unit.at)?;
        let expected = match (committed, deployed) {
            (false, _) => units.iter().find(|other| other.at == unit.at).unwrap().held,
            (true, Some(content)) => content,
            (true, None) => Content::Absent,
        };
        if found != expected {
            return Err(format!(
                "{} holds {found:?}, expected {expected:?} (committed: {committed})",
                unit.at
            ));
        }
    }
    Ok(())
}

/// The apply machine's writes go through the leaf protocol over MemFs: every
/// kill and power-loss cut of the apply, every cut of the recovery after it,
/// then recovery to the end, leaves either the generation applied from or
/// the one applied — exactly, per destination — and a release of a
/// destination a deploy of the run claims is never written (FND-3).
#[test]
fn apply_survives_every_crash_and_every_recovery_crash() {
    let units = [
        // created into absence
        Applied {
            at: "a",
            held: Content::Absent,
            deploy: Some(ONE),
        },
        // gripsack's own write replaced
        Applied {
            at: "b",
            held: TWO,
            deploy: Some(MID),
        },
        // FND-3's shape: a deploy already satisfied (the re-spelled name
        // holds gripsack's own write), then the old spelling's release —
        // the same destination, so it is never written
        Applied {
            at: "d",
            held: ONE,
            deploy: Some(ONE),
        },
        // releases come after every deploy: gripsack's own link released
        Applied {
            at: "c",
            held: Content::Link("old"),
            deploy: None,
        },
        Applied {
            at: "d",
            held: ONE,
            deploy: None,
        },
    ];
    let nested = RefCell::new(Exploration::default());
    let recover_with = |fs: &MemFs| run_recovery(fs).map(drop).map_err(RecoverError::into_io);
    let outer = explore(
        &apply_home(&units),
        CrashKinds::Both,
        PendingBound::default(),
        |fs| apply_run(fs, &units),
        |image, _| {
            let inner = explore(
                &image,
                CrashKinds::Both,
                PendingBound::default(),
                recover_with,
                |again, _| {
                    recover_with(&again).map_err(|e| format!("recovery failed: {e}"))?;
                    applied_consistently(&again, &units)
                },
            )
            .map_err(|error| format!("during recovery: {error}"))?;
            let mut nested = nested.borrow_mut();
            nested.cuts += inner.cuts;
            nested.images += inner.images;
            Ok(())
        },
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let nested = nested.into_inner();
    eprintln!("apply: {outer:?}, recoveries: {nested:?}");
    assert!(outer.cuts > 30 && nested.images > outer.images);
}
