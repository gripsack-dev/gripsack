//! The recover driver (plan/0056 §4, F2 step 2): runs the kernel's recover
//! machine over any [`Storage`].
//!
//! The kernel decides; this loop executes one command at a time and feeds
//! the result back as the next event. What the driver owns is exactly what
//! the kernel cannot: computing [`Live`] from one observation, and the
//! re-check before every act. For [`Command::Mutate`] and [`Command::Seal`]
//! the driver stages what it can first (the restored file is written and
//! synced under a temporary name), then observes the destination again by
//! name and compares it with the command's `expect` using the kernel's own
//! [`Live::same`]. A mismatch answers [`Failure::Drift`] and nothing is
//! touched; otherwise the act follows at once and the destination is
//! observed a third time for [`Event::Landed`]. Between that re-check and
//! the act there is still a window (S5): an external write landing there is
//! overwritten. It is the residual risk the claim list names; the MemFs
//! tests measure it.
//!
//! The journal's wire format, the destination resolution and the legacy
//! directory domain stay with the caller, behind [`RecoverHost`].
#![forbid(unsafe_code)]
use crate::storage::{Name, ObjectKind, Permissions, Storage};
use gripsack_kernel::recover::{
    Command, Delegated, Effect, EntryFacts, Event, Failure, Ident, JournalFacts, JournalName, Live,
    Outcome, Prior, RecoverState, Refusal, Scope, recover_step,
};
use gripsack_kernel::selection::SelectionIdentity;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// What a restore puts back, in host terms. The kernel's [`Prior`] is the
/// identity of this; the landed observation is checked against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreSource {
    Absent,
    File { bytes: Vec<u8>, mode: Permissions },
    Link { target: PathBuf },
}

/// Where a leaf entry lives: its pinned parent, its name there, and the
/// prior to put back.
#[derive(Debug)]
pub struct LeafSite<D> {
    pub parent: D,
    pub name: Name,
    pub restore: RestoreSource,
}

/// One journal entry as the host admitted it. `site` is `None` exactly when
/// the facts route the entry to the directory domain.
#[derive(Debug)]
pub struct HostEntry<D> {
    pub facts: EntryFacts,
    pub site: Option<LeafSite<D>>,
}

/// What the destination held at an observation, in more detail than
/// [`Live`]: the doctor view and refusals name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Absent,
    File,
    Link,
    /// A symlink whose target is not UTF-8 (no journal identity).
    UndecodableLink,
    Directory,
    /// A FIFO, socket or device.
    Special,
}

impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Found::Absent => "nothing (absent)",
            Found::File => "a regular file",
            Found::Link => "a symlink",
            Found::UndecodableLink => "a symlink with a non-UTF-8 target",
            Found::Directory => "a directory",
            Found::Special => "a special file (FIFO, socket or device)",
        })
    }
}

/// The world the driver acts in. Everything here is the caller's ring-2/3
/// knowledge: wire decoding, destination resolution, identity hashing and
/// the legacy directory domain.
pub trait RecoverHost {
    type Storage: Storage;

    fn storage(&self) -> &Self::Storage;

    /// Admit the journal: `None` when there is no journal at all.
    fn read_journal(&mut self) -> io::Result<Option<JournalFacts>>;

    /// The `current` selection pointer, admitted.
    fn read_current(&mut self) -> io::Result<Option<SelectionIdentity>>;

    /// Admit entry `index` (in the order the journal facts promised: reverse
    /// directory roots first) and resolve its pinned parent.
    fn read_entry(&mut self, index: u32) -> io::Result<HostEntry<<Self::Storage as Storage>::Dir>>;

    /// The identity of a regular file with these bytes and permissions.
    fn file_ident(&self, bytes: &[u8], mode: Permissions) -> Ident;

    /// The identity of a symlink with this target; `None` when the target
    /// has no journal identity (not UTF-8).
    fn link_ident(&self, target: &Path) -> Option<Ident>;

    /// A sibling name for staging a restore of `name` (must not collide
    /// with live user files; a stale one from a crash is replaced).
    fn staging_name(&self, name: &Name, link: bool) -> Name;

    /// Run one legacy directory-domain step.
    fn delegate(&mut self, step: Delegated) -> io::Result<()>;

    /// The pinned journal directory.
    fn journal_dir(&self) -> io::Result<<Self::Storage as Storage>::Dir>;

    /// Record `index` of the cleanup (entries first, then directory
    /// records), as a name in the journal directory.
    fn record_name(&self, index: u32) -> io::Result<Name>;

    fn marker_name(&self) -> Name;

    /// The directory holding the `current` pointer.
    fn selection_dir(&self) -> io::Result<<Self::Storage as Storage>::Dir>;
}

/// One settled entry, for the caller's report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The prior was put back.
    Restored,
    /// The prior was already live; it was sealed durable.
    Unchanged,
    /// A foreign object stands in place of the run's write; it was kept.
    Kept,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub entry: u32,
    pub settled: Settled,
}

#[derive(Debug)]
pub struct Recovery {
    pub outcome: Outcome,
    pub notes: Vec<Note>,
}

/// Why the driver stopped.
#[derive(Debug)]
pub enum RecoverError {
    /// The kernel refused. `found` is the last observation of the entry the
    /// refusal names, when there is one.
    Refused {
        refusal: Refusal,
        found: Option<Found>,
    },
    /// A command failed; the kernel aborted on it. The original error.
    Io(io::Error),
}

impl fmt::Display for RecoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoverError::Refused { refusal, found } => {
                write!(f, "recovery refused: {refusal:?}")?;
                if let Some(found) = found {
                    write!(f, " (found {found})")?;
                }
                Ok(())
            }
            RecoverError::Io(error) => write!(f, "recovery failed: {error}"),
        }
    }
}

impl std::error::Error for RecoverError {}

impl RecoverError {
    /// The underlying error when a command failed.
    pub fn io(&self) -> Option<&io::Error> {
        match self {
            RecoverError::Io(error) => Some(error),
            RecoverError::Refused { .. } => None,
        }
    }

    /// As an `io::Error`, for callers that only propagate.
    pub fn into_io(self) -> io::Error {
        match self {
            RecoverError::Io(error) => error,
            refused => io::Error::other(refused.to_string()),
        }
    }
}

/// Observe one destination: [`Live`] for the kernel, [`Found`] for people.
pub fn observe<H: RecoverHost>(
    host: &H,
    parent: &<H::Storage as Storage>::Dir,
    name: &Name,
) -> io::Result<(Live, Found)> {
    observe_with(
        host.storage(),
        parent,
        name,
        |bytes, mode| host.file_ident(bytes, mode),
        |target| host.link_ident(target),
    )
}

/// [`observe`] without a host, for read-only inspection: one `lstat`, then
/// the content of a file or a link, digested by the given identities.
pub fn observe_with<S: Storage>(
    storage: &S,
    parent: &S::Dir,
    name: &Name,
    file_ident: impl FnOnce(&[u8], Permissions) -> Ident,
    link_ident: impl FnOnce(&Path) -> Option<Ident>,
) -> io::Result<(Live, Found)> {
    let metadata = match storage.metadata(parent, name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Live::Absent, Found::Absent));
        }
        Err(error) => return Err(error),
    };
    Ok(match metadata.kind {
        ObjectKind::File => {
            let file = storage.open_file(parent, name)?;
            let bytes = storage.read_file(&file)?;
            (
                Live::Obj(file_ident(&bytes, metadata.permissions)),
                Found::File,
            )
        }
        ObjectKind::Symlink => match link_ident(&storage.read_link(parent, name)?) {
            Some(ident) => (Live::Obj(ident), Found::Link),
            None => (Live::Unsupported, Found::UndecodableLink),
        },
        ObjectKind::Dir => (Live::Unsupported, Found::Directory),
        ObjectKind::Other => (Live::Unsupported, Found::Special),
    })
}

/// The entry under decision, as the driver holds it between commands.
struct Current<D> {
    index: u32,
    site: Option<LeafSite<D>>,
    found: Option<Found>,
}

/// Run recovery to its end: a finished outcome with one note per settled
/// entry, or the refusal/failure that stopped it (the journal retained).
pub fn recover<H: RecoverHost>(host: &mut H) -> Result<Recovery, RecoverError> {
    let mut state = RecoverState::new();
    let mut event = Event::Start;
    let mut notes = Vec::new();
    let mut current: Option<Current<<H::Storage as Storage>::Dir>> = None;
    let mut failure: Option<io::Error> = None;
    loop {
        let (next, command) = recover_step(state, event);
        state = next;
        let result = execute(host, command, &mut current, &mut notes);
        event = match result {
            Step::Event(event) => event,
            Step::Failed(error) => {
                let errno = error.raw_os_error().unwrap_or(0);
                failure = Some(error);
                Event::Failed(Failure::Io(errno))
            }
            Step::Finished(outcome) => return Ok(Recovery { outcome, notes }),
            Step::Aborted(refusal) => {
                return Err(match (refusal, failure) {
                    (Refusal::Failed, Some(error)) => RecoverError::Io(error),
                    (refusal, _) => RecoverError::Refused {
                        refusal,
                        found: current.and_then(|current| current.found),
                    },
                });
            }
        };
    }
}

enum Step {
    Event(Event),
    Failed(io::Error),
    Finished(Outcome),
    Aborted(Refusal),
}

fn site<D>(current: &Option<Current<D>>, entry: u32) -> io::Result<&LeafSite<D>> {
    match current {
        Some(Current {
            index,
            site: Some(site),
            ..
        }) if *index == entry => Ok(site),
        _ => Err(io::Error::other(format!(
            "recover machine named entry {entry}, which the driver has not read"
        ))),
    }
}

fn execute<H: RecoverHost>(
    host: &mut H,
    command: Command,
    current: &mut Option<Current<<H::Storage as Storage>::Dir>>,
    notes: &mut Vec<Note>,
) -> Step {
    let outcome: io::Result<Event> = match command {
        Command::Finish(outcome) => return Step::Finished(outcome),
        Command::Abort(refusal) => return Step::Aborted(refusal),
        Command::ReadJournal => host.read_journal().map(|journal| match journal {
            Some(journal) => Event::JournalRead(journal),
            None => Event::NoJournal,
        }),
        Command::ReadCurrent => host.read_current().map(Event::CurrentRead),
        Command::ReadEntry(index) => host.read_entry(index).map(|entry| {
            *current = Some(Current {
                index,
                site: entry.site,
                found: None,
            });
            Event::EntryRead(entry.facts)
        }),
        Command::Observe { entry, dest } => site(current, entry)
            .and_then(|site| observe(host, &site.parent, &site.name))
            .map(|(live, found)| {
                if let Some(current) = current.as_mut() {
                    current.found = Some(found);
                }
                Event::Observed { dest, live }
            }),
        Command::Mutate {
            entry,
            expect,
            effect: Effect::Restore(prior),
            ..
        } => site(current, entry).and_then(|site| {
            let landed = restore(host, site, &expect, &prior)?;
            if landed.is_some() {
                notes.push(Note {
                    entry,
                    settled: Settled::Restored,
                });
            }
            Ok(match landed {
                Some(live) => Event::Landed { live },
                None => Event::Failed(Failure::Drift),
            })
        }),
        Command::Seal {
            entry,
            expect,
            prior,
            ..
        } => site(current, entry).and_then(|site| {
            let sealed = seal(host, site, &expect, &prior)?;
            if sealed {
                notes.push(Note {
                    entry,
                    settled: Settled::Unchanged,
                });
                Ok(Event::Done)
            } else {
                Ok(Event::Failed(Failure::Drift))
            }
        }),
        Command::Preserve { entry, .. } => {
            notes.push(Note {
                entry,
                settled: Settled::Kept,
            });
            Ok(Event::Done)
        }
        Command::Delegate(step) => host.delegate(step).map(|()| Event::Done),
        Command::Unlink(name) => unlink(host, name).map(|()| Event::Done),
        Command::Barrier(scope) => barrier(host, scope).map(|()| Event::Done),
    };
    match outcome {
        Ok(event) => Step::Event(event),
        Err(error) => Step::Failed(error),
    }
}

/// The re-check: the destination, observed again by name, is still what
/// the kernel decided on.
fn unchanged<H: RecoverHost>(
    host: &H,
    site: &LeafSite<<H::Storage as Storage>::Dir>,
    expect: &Live,
) -> io::Result<bool> {
    let (live, _) = observe(host, &site.parent, &site.name)?;
    Ok(live.same(expect))
}

/// Put the prior back under the re-check. `None` when the re-check failed
/// (nothing was changed), otherwise the landed observation.
fn restore<H: RecoverHost>(
    host: &H,
    site: &LeafSite<<H::Storage as Storage>::Dir>,
    expect: &Live,
    prior: &Prior,
) -> io::Result<Option<Live>> {
    let storage = host.storage();
    let parent = &site.parent;
    match (&site.restore, prior) {
        (RestoreSource::Absent, Prior::Absent) => {
            if !unchanged(host, site, expect)? {
                return Ok(None);
            }
            match storage.unlink(parent, &site.name) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        (RestoreSource::File { bytes, mode }, Prior::Obj(Ident::File(_))) => {
            // `atomic_write_with_mode`: temp → write → exact mode → fsync →
            // (re-check) → rename → fsync the parent. The mode rides the
            // write, so a restored 0600 secret never exists wider.
            let staged = host.staging_name(&site.name, false);
            let mut file = match storage.create_file(parent, &staged, *mode) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    storage.unlink(parent, &staged)?;
                    storage.create_file(parent, &staged, *mode)?
                }
                created => created?,
            };
            storage.write(&mut file, bytes)?;
            storage.set_mode(&file, *mode)?;
            storage.sync_file(&file)?;
            if !unchanged(host, site, expect)? {
                storage.unlink(parent, &staged)?;
                return Ok(None);
            }
            storage.rename(parent, &staged, parent, &site.name)?;
        }
        (RestoreSource::Link { target }, Prior::Obj(Ident::Link(_))) => {
            // `symlink_replace`: temp link → (re-check) → rename → fsync.
            let staged = host.staging_name(&site.name, true);
            match storage.unlink(parent, &staged) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            storage.symlink(parent, &staged, target)?;
            if !unchanged(host, site, expect)? {
                storage.unlink(parent, &staged)?;
                return Ok(None);
            }
            if let Err(error) = storage.rename(parent, &staged, parent, &site.name) {
                let _ = storage.unlink(parent, &staged);
                return Err(error);
            }
        }
        _ => {
            return Err(io::Error::other(
                "journal entry's restore source does not match its prior identity",
            ));
        }
    }
    storage.sync_dir(parent)?;
    let (live, _) = observe(host, parent, &site.name)?;
    Ok(Some(live))
}

/// `seal_observed_prior` under the re-check: a file barrier for regular
/// bytes, then the parent's namespace barrier. `false` when the re-check
/// failed.
fn seal<H: RecoverHost>(
    host: &H,
    site: &LeafSite<<H::Storage as Storage>::Dir>,
    expect: &Live,
    prior: &Prior,
) -> io::Result<bool> {
    if !unchanged(host, site, expect)? {
        return Ok(false);
    }
    let storage = host.storage();
    if let Prior::Obj(Ident::File(_)) = prior {
        let file = storage.open_file(&site.parent, &site.name)?;
        storage.sync_file(&file)?;
    }
    storage.sync_dir(&site.parent)?;
    Ok(true)
}

fn unlink<H: RecoverHost>(host: &H, name: JournalName) -> io::Result<()> {
    let journal = host.journal_dir()?;
    let storage = host.storage();
    match name {
        JournalName::Record(index) => storage.unlink(&journal, &host.record_name(index)?),
        // the marker may already be gone (an earlier cleanup's second
        // half, or a legacy run without one)
        JournalName::Marker => match storage.unlink(&journal, &host.marker_name()) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    }
}

fn barrier<H: RecoverHost>(host: &H, scope: Scope) -> io::Result<()> {
    let directory = match scope {
        Scope::Journal => host.journal_dir()?,
        Scope::Selection => host.selection_dir()?,
    };
    host.storage().sync_dir(&directory)
}
