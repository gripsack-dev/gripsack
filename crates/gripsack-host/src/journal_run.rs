//! The journal-run driver (plan/0056 §4, F2 step 3): runs the kernel's
//! journal-run machine, and the leaf effect protocol inside it, over any
//! [`Storage`].
//!
//! The kernel decides the order — the run marker's publication, then per
//! destination: observe, capture the prior, publish the entry, mutate — and
//! when journal evidence may go. This driver executes one command at a time
//! and owns what the kernel cannot: the bytes of each record (asked of the
//! [`RunHost`]), computing [`Live`] from one observation, and the re-check
//! before every act. For [`Command::CapturePrior`] and [`Command::Mutate`] the
//! destination is observed again by name and compared with the command's
//! `expect` using the kernel's own [`Live::same`]; a mismatch answers
//! [`Failure::Drift`] and nothing is touched. A mutation stages what it can
//! first (a file is written and synced under a temporary name) and re-checks
//! immediately before the rename or unlink; between that re-check and the
//! act there is still a window (S5), the residual risk the claim list names.
//!
//! The machine lives as long as the run: [`RunDriver::begin`] publishes the
//! marker, [`RunDriver::leaf`] mutates one destination, [`RunDriver::commit`]
//! stops at the flip (the caller flips) and [`RunDriver::flipped`] /
//! [`RunDriver::end`] clean up.
use crate::recover::Found;
use crate::storage::{Name, ObjectKind, Permissions, Storage};
use gripsack_kernel::journal::PublicationAction;
use gripsack_kernel::journal_run::{
    Command, Event, Outcome, Record, Refusal, RunState, Settle, UnitFacts, run_step,
};
use gripsack_kernel::recover::{Failure, Ident, JournalName, Live};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Mode of every journal record (private state).
pub const RECORD_MODE: Permissions = Permissions::new(0o600);

/// A destination as one observation found it, with the content the kernel's
/// identity was computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    Absent,
    File {
        bytes: Vec<u8>,
        mode: Permissions,
    },
    Link {
        target: PathBuf,
    },
    /// No journal identity: a directory, a special file, a link whose
    /// target is not UTF-8.
    Unsupported(Found),
}

impl Observation {
    /// One `lstat`, then the content of a file or a link.
    pub fn of<S: Storage>(storage: &S, parent: &S::Dir, name: &Name) -> io::Result<Self> {
        let metadata = match storage.metadata(parent, name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::Absent),
            Err(error) => return Err(error),
        };
        Ok(match metadata.kind {
            ObjectKind::File => {
                let file = storage.open_file(parent, name)?;
                Self::File {
                    bytes: storage.read_file(&file)?,
                    mode: metadata.permissions,
                }
            }
            ObjectKind::Symlink => {
                let target = storage.read_link(parent, name)?;
                if target.to_str().is_some() {
                    Self::Link { target }
                } else {
                    Self::Unsupported(Found::UndecodableLink)
                }
            }
            ObjectKind::Dir => Self::Unsupported(Found::Directory),
            ObjectKind::Other => Self::Unsupported(Found::Special),
        })
    }

    pub fn found(&self) -> Found {
        match self {
            Self::Absent => Found::Absent,
            Self::File { .. } => Found::File,
            Self::Link { .. } => Found::Link,
            Self::Unsupported(found) => *found,
        }
    }
}

/// What a mutation puts at the destination.
pub enum Install<'a> {
    File {
        bytes: Vec<u8>,
        mode: Permissions,
    },
    Link {
        target: PathBuf,
    },
    Remove,
    /// Rewrite the destination's latest bytes (a managed block's upsert or
    /// removal): given the re-checked file's bytes and mode (empty and
    /// `None` when absent), the new bytes and mode, or `None` to remove it.
    Splice(Box<Splice<'a>>),
}

pub type Splice<'a> =
    dyn FnOnce(&[u8], Option<Permissions>) -> io::Result<Option<(Vec<u8>, Permissions)>> + 'a;

impl fmt::Debug for Install<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Install::File { bytes, mode } => write!(f, "File({} bytes, {mode})", bytes.len()),
            Install::Link { target } => write!(f, "Link({})", target.display()),
            Install::Remove => f.write_str("Remove"),
            Install::Splice(_) => f.write_str("Splice"),
        }
    }
}

/// One destination the run mutates: the kernel's facts, the pinned parent
/// and name, and what to install.
#[derive(Debug)]
pub struct Leaf<'a, D> {
    pub facts: UnitFacts,
    pub parent: D,
    pub name: Name,
    pub install: Install<'a>,
}

/// A journal record: its name in the journal directory and its bytes.
#[derive(Debug, Clone)]
pub struct JournalRecord {
    pub name: Name,
    pub bytes: Vec<u8>,
}

/// The caller's ring-2/3 knowledge: the wire of the marker and the entries,
/// prior evidence storage, identity hashing, the legacy directory domain.
pub trait RunHost {
    type Storage: Storage;

    fn storage(&self) -> &Self::Storage;

    /// The pinned journal directory.
    fn journal_dir(&self) -> io::Result<<Self::Storage as Storage>::Dir>;

    /// The run marker's name in the journal directory.
    fn marker_name(&self) -> Name;

    /// The run marker's bytes (asked once, when it is staged).
    fn marker_bytes(&self) -> io::Result<Vec<u8>>;

    /// The identity of a regular file with these bytes and permissions.
    fn file_ident(&self, bytes: &[u8], mode: Permissions) -> Ident;

    /// The identity of a symlink with this target; `None` when the target
    /// has no journal identity (not UTF-8).
    fn link_ident(&self, target: &Path) -> Option<Ident>;

    /// A sibling name for staging `name` (must not collide with user files;
    /// a stale one from a crash is replaced).
    fn staging_name(&self, name: &Name, link: bool) -> Name;

    /// Make the destination's prior durable evidence (a regular file's
    /// bytes and mode; a link and an absence travel in the entry) and return
    /// the entry that records it with the unit's intent.
    fn capture(&mut self, facts: &UnitFacts, prior: &Observation) -> io::Result<JournalRecord>;

    /// Settle the legacy directory domain for this outcome; the number of
    /// journal records (entries, then directory records) the cleanup removes.
    fn settle(&mut self, settle: Settle) -> io::Result<u32>;

    /// Cleanup record `index`, as a name in the journal directory.
    fn record_name(&self, index: u32) -> io::Result<Name>;
}

/// Why the driver stopped.
#[derive(Debug)]
pub enum RunError {
    /// The kernel refused. `found` is the last observation of the unit the
    /// refusal names, when there is one.
    Refused {
        refusal: Refusal,
        found: Option<Found>,
    },
    /// A command failed; the kernel aborted on it. The original error.
    Io(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Refused { refusal, found } => {
                write!(f, "journal run refused: {refusal:?}")?;
                if let Some(found) = found {
                    write!(f, " (found {found})")?;
                }
                Ok(())
            }
            RunError::Io(error) => write!(f, "journal run failed: {error}"),
        }
    }
}

impl std::error::Error for RunError {}

impl RunError {
    pub fn refusal(&self) -> Option<Refusal> {
        match self {
            RunError::Refused { refusal, .. } => Some(*refusal),
            RunError::Io(_) => None,
        }
    }

    /// As an `io::Error`, for callers that only propagate.
    pub fn into_io(self) -> io::Error {
        match self {
            RunError::Io(error) => error,
            refused => io::Error::other(refused.to_string()),
        }
    }
}

/// Where the driver stopped without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Ready,
    Flip,
    Finished(Outcome),
}

struct Staged<F> {
    file: F,
    temporary: Name,
    name: Name,
}

impl<F> fmt::Debug for Staged<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Staged")
            .field("temporary", &self.temporary)
            .field("name", &self.name)
            .finish()
    }
}

/// The run's machine and what the driver holds between its commands.
pub struct RunDriver<S: Storage> {
    state: RunState,
    staged: Option<Staged<S::File>>,
    entry: Option<JournalRecord>,
    found: Option<Found>,
}

impl<S: Storage> fmt::Debug for RunDriver<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunDriver")
            .field("state", &self.state)
            .field("staged", &self.staged)
            .finish()
    }
}

impl<S: Storage> RunDriver<S> {
    /// Declare the run: the marker is durably published before this returns.
    pub fn begin<H: RunHost<Storage = S>>(host: &mut H) -> Result<Self, RunError> {
        let mut driver = Self {
            state: RunState::new(),
            staged: None,
            entry: None,
            found: None,
        };
        match driver.drive(host, Event::Start, None)? {
            Stop::Ready => Ok(driver),
            other => Err(unexpected(other)),
        }
    }

    /// Mutate one destination under the leaf protocol: observe, capture the
    /// prior, publish the entry, then act under the re-check. Returns once
    /// the intended object landed.
    pub fn leaf<H: RunHost<Storage = S>>(
        &mut self,
        host: &mut H,
        leaf: Leaf<'_, S::Dir>,
    ) -> Result<(), RunError> {
        self.found = None;
        let facts = leaf.facts;
        match self.drive(host, Event::Unit(facts), Some(leaf))? {
            Stop::Ready => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// The run's units are done: returns when the caller must flip.
    pub fn commit<H: RunHost<Storage = S>>(&mut self, host: &mut H) -> Result<(), RunError> {
        match self.drive(host, Event::Commit, None)? {
            Stop::Flip => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// The flip is durable: settle and remove the journal evidence.
    pub fn flipped<H: RunHost<Storage = S>>(&mut self, host: &mut H) -> Result<(), RunError> {
        match self.drive(host, Event::Done, None)? {
            Stop::Finished(Outcome::Committed) => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// The flip failed: the machine aborts (recovery compensates).
    pub fn flip_failed(&mut self, error: &io::Error) {
        let errno = error.raw_os_error().unwrap_or(0);
        let (state, _) = run_step(self.state, Event::Failed(Failure::Io(errno)));
        self.state = state;
    }

    /// End a run that journaled no destination: remove its marker.
    pub fn end<H: RunHost<Storage = S>>(&mut self, host: &mut H) -> Result<(), RunError> {
        match self.drive(host, Event::End, None)? {
            Stop::Finished(Outcome::Ended) => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    fn drive<H: RunHost<Storage = S>>(
        &mut self,
        host: &mut H,
        first: Event,
        mut leaf: Option<Leaf<'_, S::Dir>>,
    ) -> Result<Stop, RunError> {
        let mut event = first;
        let mut failure: Option<io::Error> = None;
        loop {
            let (state, command) = run_step(self.state, event);
            self.state = state;
            event = match command {
                Command::Ready => return Ok(Stop::Ready),
                Command::Flip => return Ok(Stop::Flip),
                Command::Finish(outcome) => return Ok(Stop::Finished(outcome)),
                Command::Abort(refusal) => {
                    return Err(match (refusal, failure) {
                        (Refusal::Failed, Some(error)) => RunError::Io(error),
                        (refusal, _) => RunError::Refused {
                            refusal,
                            found: self.found,
                        },
                    });
                }
                command => match self.execute(host, command, &mut leaf) {
                    Ok(event) => event,
                    Err(error) => {
                        let errno = error.raw_os_error().unwrap_or(0);
                        failure = Some(error);
                        Event::Failed(Failure::Io(errno))
                    }
                },
            };
        }
    }

    fn execute<H: RunHost<Storage = S>>(
        &mut self,
        host: &mut H,
        command: Command,
        leaf: &mut Option<Leaf<'_, S::Dir>>,
    ) -> io::Result<Event> {
        match command {
            Command::Publish { record, action } => {
                self.publish(host, record, action)?;
                Ok(Event::Done)
            }
            Command::Observe { dest, .. } => {
                let leaf = leaf_of(leaf)?;
                let observation = Observation::of(host.storage(), &leaf.parent, &leaf.name)?;
                self.found = Some(observation.found());
                let live = live(host, &observation);
                Ok(Event::Observed { dest, live })
            }
            Command::CapturePrior { expect, .. } => {
                let leaf = leaf_of(leaf)?;
                let observation = Observation::of(host.storage(), &leaf.parent, &leaf.name)?;
                if !live(host, &observation).same(&expect) {
                    return Ok(Event::Failed(Failure::Drift));
                }
                self.entry = Some(host.capture(&leaf.facts, &observation)?);
                Ok(Event::Done)
            }
            Command::Mutate { expect, .. } => {
                let leaf = leaf.take().ok_or_else(no_leaf)?;
                Ok(match install(host, leaf, &expect)? {
                    Some(live) => Event::Landed { live },
                    None => Event::Failed(Failure::Drift),
                })
            }
            Command::Settle(settle) => host
                .settle(settle)
                .map(|records| Event::Settled { records }),
            Command::Unlink(name) => {
                let journal = host.journal_dir()?;
                let storage = host.storage();
                match name {
                    JournalName::Record(index) => {
                        storage.unlink(&journal, &host.record_name(index)?)?
                    }
                    // a retried cleanup may find it gone already
                    JournalName::Marker => match storage.unlink(&journal, &host.marker_name()) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        other => other?,
                    },
                }
                Ok(Event::Done)
            }
            Command::Barrier => {
                host.storage().sync_dir(&host.journal_dir()?)?;
                Ok(Event::Done)
            }
            Command::Ready | Command::Flip | Command::Finish(_) | Command::Abort(_) => {
                Err(io::Error::other("the driver loop handles this command"))
            }
        }
    }

    /// One step of a record's publication (`StagedFileWrite`'s order, as
    /// the kernel issues it).
    fn publish<H: RunHost<Storage = S>>(
        &mut self,
        host: &H,
        record: Record,
        action: PublicationAction,
    ) -> io::Result<()> {
        let storage = host.storage();
        let journal = host.journal_dir()?;
        match action {
            PublicationAction::Stage => {
                let JournalRecord { name, bytes } = match record {
                    Record::Marker => JournalRecord {
                        name: host.marker_name(),
                        bytes: host.marker_bytes()?,
                    },
                    Record::Entry => self.entry.take().ok_or_else(|| {
                        io::Error::other("the entry is published before its prior was captured")
                    })?,
                };
                let temporary = host.staging_name(&name, false);
                let mut file = create_replacing(storage, &journal, &temporary, RECORD_MODE)?;
                storage.write(&mut file, &bytes)?;
                storage.set_mode(&file, RECORD_MODE)?;
                self.staged = Some(Staged {
                    file,
                    temporary,
                    name,
                });
            }
            PublicationAction::SyncFile => storage.sync_file(&self.staged()?.file)?,
            PublicationAction::PublishName => {
                let staged = self.staged()?;
                storage.rename(&journal, &staged.temporary, &journal, &staged.name)?;
            }
            PublicationAction::SyncNamespace => {
                storage.sync_dir(&journal)?;
                self.staged = None;
            }
            PublicationAction::Complete | PublicationAction::Refused => {
                return Err(io::Error::other("not a publication effect"));
            }
        }
        Ok(())
    }

    fn staged(&self) -> io::Result<&Staged<S::File>> {
        self.staged
            .as_ref()
            .ok_or_else(|| io::Error::other("no record is staged"))
    }
}

fn unexpected(stop: Stop) -> RunError {
    RunError::Io(io::Error::other(format!(
        "the journal run stopped unexpectedly at {stop:?}"
    )))
}

fn no_leaf() -> io::Error {
    io::Error::other("the journal run named a unit the driver was not given")
}

fn leaf_of<'l, 'a, D>(leaf: &'l Option<Leaf<'a, D>>) -> io::Result<&'l Leaf<'a, D>> {
    leaf.as_ref().ok_or_else(no_leaf)
}

/// The kernel's identity of an observation.
pub fn live<H: RunHost>(host: &H, observation: &Observation) -> Live {
    match observation {
        Observation::Absent => Live::Absent,
        Observation::File { bytes, mode } => Live::Obj(host.file_ident(bytes, *mode)),
        Observation::Link { target } => match host.link_ident(target) {
            Some(ident) => Live::Obj(ident),
            None => Live::Unsupported,
        },
        Observation::Unsupported(_) => Live::Unsupported,
    }
}

/// Exclusive create of a staging name, replacing a stale one from a crash.
fn create_replacing<S: Storage>(
    storage: &S,
    dir: &S::Dir,
    name: &Name,
    mode: Permissions,
) -> io::Result<S::File> {
    match storage.create_file(dir, name, mode) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            storage.unlink(dir, name)?;
            storage.create_file(dir, name, mode)
        }
        created => created,
    }
}

/// The re-check: the destination, observed again by name, is still what
/// the kernel decided on.
fn unchanged<H: RunHost>(
    host: &H,
    parent: &<H::Storage as Storage>::Dir,
    name: &Name,
    expect: &Live,
) -> io::Result<Option<Observation>> {
    let observation = Observation::of(host.storage(), parent, name)?;
    Ok(live(host, &observation).same(expect).then_some(observation))
}

/// Act under the re-check. `None` when the re-check failed (nothing was
/// changed), otherwise the landed observation.
fn install<H: RunHost>(
    host: &H,
    leaf: Leaf<'_, <H::Storage as Storage>::Dir>,
    expect: &Live,
) -> io::Result<Option<Live>> {
    let storage = host.storage();
    let Leaf {
        parent,
        name,
        install,
        ..
    } = leaf;
    let content = match install {
        Install::Splice(splice) => {
            let Some(latest) = unchanged(host, &parent, &name, expect)? else {
                return Ok(None);
            };
            let spliced = match &latest {
                Observation::Absent => splice(&[], None)?,
                Observation::File { bytes, mode } => splice(bytes, Some(*mode))?,
                _ => {
                    return Err(io::Error::other(
                        "a managed block's host is not a regular file",
                    ));
                }
            };
            match spliced {
                Some((bytes, mode)) => Install::File { bytes, mode },
                None => Install::Remove,
            }
        }
        other => other,
    };
    match content {
        Install::File { bytes, mode } => {
            // `atomic_write_with_mode`: temp → write → exact mode → fsync →
            // (re-check) → rename → fsync the parent.
            let staged = host.staging_name(&name, false);
            let mut file = create_replacing(storage, &parent, &staged, mode)?;
            storage.write(&mut file, &bytes)?;
            storage.set_mode(&file, mode)?;
            storage.sync_file(&file)?;
            if unchanged(host, &parent, &name, expect)?.is_none() {
                storage.unlink(&parent, &staged)?;
                return Ok(None);
            }
            storage.rename(&parent, &staged, &parent, &name)?;
        }
        Install::Link { target } => {
            // `symlink_replace`: temp link → (re-check) → rename → fsync.
            let staged = host.staging_name(&name, true);
            match storage.unlink(&parent, &staged) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            storage.symlink(&parent, &staged, &target)?;
            if unchanged(host, &parent, &name, expect)?.is_none() {
                storage.unlink(&parent, &staged)?;
                return Ok(None);
            }
            if let Err(error) = storage.rename(&parent, &staged, &parent, &name) {
                let _ = storage.unlink(&parent, &staged);
                return Err(error);
            }
        }
        Install::Remove => {
            if unchanged(host, &parent, &name, expect)?.is_none() {
                return Ok(None);
            }
            match storage.unlink(&parent, &name) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Install::Splice(_) => unreachable!("a splice resolved to its content above"),
    }
    storage.sync_dir(&parent)?;
    Ok(Some(live(host, &Observation::of(storage, &parent, &name)?)))
}
