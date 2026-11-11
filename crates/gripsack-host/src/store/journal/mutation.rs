//! The journal run through the kernel's machine (plan/0056 F2 step 3).
//!
//! `gripsack_kernel::journal_run` decides the order — the marker's
//! publication, then per destination: observe, capture the prior, publish
//! the entry, mutate under a re-check — and when journal evidence may go;
//! `crate::journal_run` drives it over `RealFs`. This module is the
//! [`RunHost`]: the marker and entry wire, prior blobs, identity hashing, and
//! the legacy directory domain's part of settling a run.
use super::marker::{JournalRun, RUN_MARKER};
use super::recover_kernel::{file_ident, kernel_ident, link_ident, real_dir, staging_name};
use super::storage::Journal;
use super::{Entry, Intended, ObjectIdentity, Prior, PriorSerde, entry_name};
use crate::fs::Dir;
use crate::journal_run::{Install, JournalRecord, Leaf, Observation, RunError, RunHost};
use crate::storage::{Name, Permissions, RealDir, RealFs};
use gripsack_kernel::DestId;
use gripsack_kernel::journal_run::{Refusal, Settle, UnitFacts};
use gripsack_kernel::recover::{Ident, Intended as KernelIntended, Live};
use std::io;
use std::path::{Path, PathBuf};

fn name(path: &Path) -> io::Result<Name> {
    Name::new(path.as_os_str()).map_err(io::Error::from)
}

/// The leaf under way: its destination and what its entry records.
struct UnitWire {
    destination: String,
    previous: Option<Entry>,
    intended: Intended,
}

pub(super) struct StoreRunHost<'a> {
    storage: RealFs,
    journal: &'a Journal,
    home: &'a Dir,
    marker: Option<Vec<u8>>,
    unit: Option<UnitWire>,
    records: Vec<Name>,
}

impl<'a> StoreRunHost<'a> {
    pub(super) fn new(journal: &'a Journal, home: &'a Dir, home_path: &Path) -> io::Result<Self> {
        Ok(Self {
            storage: RealFs::new(home_path),
            journal,
            home,
            marker: None,
            unit: None,
            records: Vec::new(),
        })
    }

    pub(super) fn with_marker(mut self, bytes: Vec<u8>) -> Self {
        self.marker = Some(bytes);
        self
    }
}

/// A destination's prior as durable evidence: a regular file's bytes go to
/// the prior blob store (synced before this returns), a link's target and an
/// absence travel in the entry.
fn prior_evidence(home: &Dir, observation: &Observation) -> io::Result<Prior> {
    Ok(match observation {
        Observation::Absent => Prior::Absent,
        Observation::File { bytes, mode } => Prior::File {
            mode: crate::store::prior::FileMode::try_from(mode.bits())?,
            hash: crate::store::prior::store_blob(home, bytes)?,
        },
        Observation::Link { target } => Prior::Symlink {
            target: target
                .to_str()
                .ok_or_else(|| io::Error::other("a journal prior link target is not UTF-8"))?
                .to_owned(),
        },
        Observation::Unsupported(found) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("journal prior is {found}, not a regular file or symlink"),
            ));
        }
    })
}

impl RunHost for StoreRunHost<'_> {
    type Storage = RealFs;

    fn storage(&self) -> &RealFs {
        &self.storage
    }

    fn journal_dir(&self) -> io::Result<RealDir> {
        real_dir(&self.journal.directory)
    }

    fn marker_name(&self) -> Name {
        Name::new(RUN_MARKER).expect("the marker name is one component")
    }

    fn marker_bytes(&self) -> io::Result<Vec<u8>> {
        self.marker
            .clone()
            .ok_or_else(|| io::Error::other("the run marker is published only at begin_run"))
    }

    fn file_ident(&self, bytes: &[u8], mode: Permissions) -> Ident {
        file_ident(bytes, mode)
    }

    fn link_ident(&self, target: &Path) -> Option<Ident> {
        link_ident(target)
    }

    fn staging_name(&self, at: &Name, link: bool) -> Name {
        staging_name(at, link)
    }

    fn capture(&mut self, _facts: &UnitFacts, prior: &Observation) -> io::Result<JournalRecord> {
        let unit = self
            .unit
            .take()
            .ok_or_else(|| io::Error::other("no journal unit is under way"))?;
        let before = PriorSerde::from(&prior_evidence(self.home, prior)?);
        let after = unit.intended.to_serde();
        let entry = match unit.previous {
            Some(previous) => previous.advance(before, after),
            None => Entry::new(unit.destination.clone(), before, after),
        };
        Ok(JournalRecord {
            name: name(&entry_name(Path::new(&unit.destination)))?,
            bytes: serde_json::to_vec(&entry)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        })
    }

    fn settle(&mut self, settle: Settle) -> io::Result<u32> {
        let journal = self.journal;
        journal.restrict()?;
        let directories = super::directory::read_records(journal)?;
        let names: Vec<PathBuf> = match settle {
            Settle::Committed => {
                super::directory_lifecycle::committed(&directories)?;
                let mut names = Vec::new();
                for entry in journal.directory.read_dir(".")? {
                    let at = PathBuf::from(entry?.file_name());
                    if at != Path::new(RUN_MARKER) && at.extension().is_some_and(|e| e == "json") {
                        names.push(at);
                    }
                }
                names
            }
            Settle::Ended => {
                // The kernel ends only a run that journaled no destination;
                // bound directory records are the only evidence it holds.
                if directories
                    .iter()
                    .any(|(_, record)| record.direction != super::directory::Direction::Bound)
                {
                    return Err(io::Error::other(
                        "cannot end a run with a directory transition",
                    ));
                }
                for (_, record) in &directories {
                    crate::fs::fsync_dir(&record.transition.parent()?, Path::new("."))?;
                }
                directories.into_iter().map(|(path, _)| path).collect()
            }
        };
        self.records = names
            .iter()
            .map(|path| name(path))
            .collect::<io::Result<_>>()?;
        u32::try_from(self.records.len())
            .map_err(|_| io::Error::other("too many journal records to clean up"))
    }

    fn record_name(&self, index: u32) -> io::Result<Name> {
        self.records
            .get(index as usize)
            .cloned()
            .ok_or_else(|| io::Error::other(format!("no journal record {index} to clean up")))
    }
}

/// The kernel's view of an intent.
/// The kernel's identity of a journal intent (the apply machine compares the
/// op a planner built with the install it commanded).
pub fn kernel_intended(intended: &Intended) -> io::Result<KernelIntended> {
    Ok(match intended {
        Intended::Removed => KernelIntended::Removed,
        Intended::Object(identity) => KernelIntended::Obj(kernel_ident(identity)?),
    })
}

/// The kernel's view of a plan-time observation.
fn kernel_live(observed: Option<&ObjectIdentity>) -> io::Result<Live> {
    Ok(match observed {
        None => Live::Absent,
        Some(identity) => Live::Obj(kernel_ident(identity)?),
    })
}

/// A logical child is resolved through the persisted directory hierarchy,
/// never through the predecessor symlink or a replacement ancestor, even if
/// the caller opened that path.
fn resolve(
    run: &JournalRun<'_>,
    directory: Dir,
    leaf: PathBuf,
    destination: &Path,
) -> io::Result<(Dir, PathBuf)> {
    Ok(match super::directory::bound_parent(run, destination)? {
        Some(bound) => bound,
        None => (directory, leaf),
    })
}

fn utf8(destination: &Path) -> io::Result<&str> {
    destination.to_str().ok_or_else(|| {
        io::Error::other(format!(
            "destination {} is not valid UTF-8 — refusing to journal it (the object is untouched)",
            destination.display(),
        ))
    })
}

/// Mutate one destination under the journal: the kernel's leaf protocol
/// observes it, refuses unless it is still `expected` (the plan's
/// observation) — and, for a repeated write, what this run's entry left
/// there — captures the prior durably, publishes the entry, then installs
/// under a re-check of the same observation and verifies that `intended`
/// landed. External writers can still race the re-check and the act (S5).
pub fn mutate(
    run: &JournalRun<'_>,
    directory: Dir,
    leaf: PathBuf,
    destination: &Path,
    expected: Option<&ObjectIdentity>,
    intended: &Intended,
    install: Install,
) -> io::Result<Live> {
    let (directory, leaf) = resolve(run, directory, leaf, destination)?;
    let utf8 = utf8(destination)?;
    let previous = match run.journal.read(&entry_name(destination)) {
        Ok(bytes) => {
            let previous = Entry::from_wire(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            if previous.dest != utf8 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "journal destination identity changed",
                ));
            }
            Some(previous)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let facts = UnitFacts {
        dest: DestId::new(0),
        expect: kernel_live(expected)?,
        intended: kernel_intended(intended)?,
        follows: previous
            .as_ref()
            .map(|entry| kernel_intended(&Intended::from_wire(&entry.after)))
            .transpose()?,
    };
    let leaf = Leaf {
        facts,
        parent: real_dir(&directory)?,
        name: name(&leaf)?,
        install,
    };
    let mut host = StoreRunHost::new(&run.journal, run.home(), run.home_path())?;
    host.unit = Some(UnitWire {
        destination: utf8.to_owned(),
        previous,
        intended: intended.clone(),
    });
    run.machine()?
        .leaf(&mut host, leaf)
        .map_err(|error| leaf_error(error, destination, intended))
}

/// A leaf refusal as the error the CLI shows.
fn leaf_error(error: RunError, destination: &Path, intended: &Intended) -> io::Error {
    let shown = destination.display();
    match &error {
        RunError::Refused { refusal, found } => match refusal {
            Refusal::Drifted { .. } => io::Error::other(format!(
                "{shown} changed between the drift decision and the mutation — aborting; re-run to retry",
            )),
            Refusal::Interleaved { .. } => io::Error::other(format!(
                "{shown} changed between journaled mutations; refusing to overwrite the intervening edit",
            )),
            Refusal::Unsupported { .. } => io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{shown} is {} — gripsack cannot journal it; move it aside to continue \
                     (it is preserved; nothing was deleted)",
                    found.map_or_else(
                        || "an object without a journal identity".to_owned(),
                        |found| found.to_string()
                    )
                ),
            ),
            Refusal::NotLanded { .. } => io::Error::other(format!(
                "{shown} did not reach its intended state (expected {intended})",
            )),
            _ => error.into_io(),
        },
        RunError::Io(_) => error.into_io(),
    }
}

/// Capture a destination's prior durably for the directory domain (a
/// retained incarnation's reactivation), refusing unless it is still
/// `expected`. Not a leaf mutation: the directory record carries the prior.
pub(super) fn capture(
    run: &JournalRun<'_>,
    directory: Dir,
    leaf: PathBuf,
    destination: &Path,
    expected: Option<&ObjectIdentity>,
) -> io::Result<Prior> {
    let (directory, leaf) = resolve(run, directory, leaf, destination)?;
    utf8(destination)?;
    let storage = RealFs::new(run.home_path());
    let observation = Observation::of(&storage, &real_dir(&directory)?, &name(&leaf)?)?;
    let live = match &observation {
        Observation::Absent => Live::Absent,
        Observation::File { bytes, mode } => Live::Obj(file_ident(bytes, *mode)),
        Observation::Link { target } => match link_ident(target) {
            Some(ident) => Live::Obj(ident),
            None => Live::Unsupported,
        },
        Observation::Unsupported(_) => Live::Unsupported,
    };
    if !live.same(&kernel_live(expected)?) {
        return Err(io::Error::other(format!(
            "{} changed between the drift decision and the mutation — aborting; re-run to retry",
            destination.display(),
        )));
    }
    prior_evidence(run.home(), &observation)
}

/// Tests only: the leaf protocol up to its durable entry, then a failure
/// where the act would be — the evidence a crash between the entry and the
/// mutation leaves. The run's machine aborts, as the process would have.
#[cfg(test)]
pub(super) fn crash_after_entry(
    run: &JournalRun<'_>,
    directory: Dir,
    leaf: PathBuf,
    destination: &Path,
    expected: Option<&ObjectIdentity>,
    intended: &Intended,
) {
    let crash = |_: &[u8], _: Option<Permissions>| Err(io::Error::other("simulated crash"));
    mutate(
        run,
        directory,
        leaf,
        destination,
        expected,
        intended,
        Install::Splice(Box::new(crash)),
    )
    .expect_err("the simulated crash stops the leaf after its entry");
}

/// Tests only: a regular file's install.
#[cfg(test)]
pub(super) fn file(bytes: &[u8], mode: u32) -> Install<'static> {
    Install::File {
        bytes: bytes.to_vec(),
        mode: Permissions::new(mode),
    }
}
