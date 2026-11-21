//! A read-only view of a pending journal, for `grip doctor` (0058 item 1).
//!
//! Recovery runs before apply, rollback and gc; when it refuses, the user
//! needs to see *which* entry blocks it, what recovery expected there, what
//! it found, and what to do. This view reads the same evidence recovery
//! reads and asks the kernel's own per-entry [`decide`] — the function the
//! recover machine uses — but changes nothing: no quarantine moves, no
//! straggler cleanup, no permission repair, no directory creation.
use super::recover_kernel::{entry_facts, expectation, file_ident, link_ident};
use super::storage::{Journal, RUN_MARKER};
use super::{Entry, PendingRecovery, pending_recovery};
use crate::fs::Dir;
use crate::recover::{Found, observe_with};
use crate::storage::{Name, RealFs};
use gripsack_kernel::Classification;
use gripsack_kernel::recover::{Decision, decide};
use gripsack_kernel::selection::{RecoveryFacts, classify};
use std::io::{self, Read};
use std::path::Path;

/// How recovery will treat the interrupted run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunVerdict {
    /// No run marker: entries are restored (a legacy-shaped run).
    Unmarked,
    /// The flip landed: the journal is cleared, deployed content stands.
    Committed,
    /// The flip never landed: every entry's prior is restored.
    Uncommitted,
    /// `current` matches neither selection: recovery refuses.
    Ambiguous,
}

/// What recovery will do with one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryOutlook {
    /// The kernel's decision against the live object.
    Decided(Decision, Found),
    /// The run committed: the entry is only cleaned up.
    Committed,
    /// The entry could not be evaluated (unreadable prior blob, parent
    /// unreachable, …): recovery will stop on it too.
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryView {
    pub dest: String,
    /// What recovery accepts there and what it would restore, for people.
    pub expected: String,
    pub outlook: EntryOutlook,
}

impl EntryView {
    /// Recovery cannot decide this entry or refuses its live identity.
    pub fn blocks(&self) -> bool {
        matches!(
            self.outlook,
            EntryOutlook::Unreadable(_) | EntryOutlook::Decided(Decision::Refuse, _)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalView {
    pub pending: PendingRecovery,
    pub verdict: RunVerdict,
    pub entries: Vec<EntryView>,
    /// Entries or markers that do not decode (they will be quarantined).
    pub unadmitted: Vec<String>,
    pub directories: usize,
}

impl JournalView {
    /// The first entry recovery refuses, if any.
    pub fn blocked(&self) -> Option<&EntryView> {
        self.entries.iter().find(|entry| entry.blocks())
    }
}

fn read_plain(journal: &Journal, name: &Path) -> io::Result<Vec<u8>> {
    let mut file = crate::fs::open_file_nofollow(&journal.directory, name)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn verdict(home: &Dir, home_path: &Path, journal: &Journal) -> io::Result<RunVerdict> {
    let bytes = match read_plain(journal, Path::new(RUN_MARKER)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(RunVerdict::Unmarked),
        result => result?,
    };
    let marker = serde_json::from_slice::<super::marker::RunMarker>(&bytes).map_err(|error| {
        io::Error::new(io::ErrorKind::InvalidData, format!("run marker: {error}"))
    })?;
    marker.admit_recovery()?;
    let current = crate::store::generations::current_selection_in(home_path, home)?;
    if let Some(selection) = &current {
        let id = selection.generation();
        crate::store::generations::read_manifest_at(home, home_path, id).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("current generation {id} manifest: {error}"),
            )
        })?;
    }
    Ok(
        match classify(RecoveryFacts {
            previous: marker.previous,
            target: marker.target,
            current,
        }) {
            Classification::Committed => RunVerdict::Committed,
            Classification::Uncommitted => RunVerdict::Uncommitted,
            Classification::Ambiguous => RunVerdict::Ambiguous,
        },
    )
}

fn observe_destination(dest: &Path) -> io::Result<(gripsack_kernel::recover::Live, Found)> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let parent = match crate::fs::open(parent) {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((gripsack_kernel::recover::Live::Absent, Found::Absent));
        }
        Err(error) => return Err(error),
    };
    let leaf = Name::new(dest.file_name().unwrap_or_default())?;
    observe_with(
        &RealFs::new("/"),
        &super::recover_kernel::real_dir(&parent)?,
        &leaf,
        file_ident,
        link_ident,
    )
}

/// Observe and decide one entry without effects. A committed cleanup needs
/// neither the live destination nor the prior bytes.
fn entry_view(index: u32, entry: &Entry, home: &Dir, committed: bool) -> EntryView {
    let observe = || -> io::Result<EntryOutlook> {
        if committed {
            return Ok(EntryOutlook::Committed);
        }
        let identity = |prior: &super::PriorSerde| {
            super::prior_identity_with(prior, |hash| {
                crate::store::prior::observe_blob(home, hash).map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("prior blob {}: {error}", hash.as_str()),
                    )
                })
            })
        };
        let prior = identity(&entry.prior)?;
        let facts = entry_facts(index, entry, prior.as_ref(), identity)?;
        let (live, found) = observe_destination(Path::new(&entry.dest))?;
        Ok(EntryOutlook::Decided(decide(&facts, &live), found))
    };
    EntryView {
        dest: entry.dest.clone(),
        expected: expectation(entry),
        outlook: observe().unwrap_or_else(|error| EntryOutlook::Unreadable(error.to_string())),
    }
}

/// The pending journal, read-only; `None` when nothing is pending.
pub fn inspect(home: &Dir, home_path: &Path) -> io::Result<Option<JournalView>> {
    let Some(pending) = pending_recovery(home)? else {
        return Ok(None);
    };
    let Some(journal) = Journal::open(home)? else {
        return Ok(None);
    };
    let verdict = verdict(home, home_path, &journal)?;
    let committed = verdict == RunVerdict::Committed;
    let mut entries = Vec::new();
    let mut unadmitted = Vec::new();
    let mut directories = 0;
    let mut names: Vec<_> = journal
        .directory
        .read_dir(".")?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<_>>()?;
    names.sort();
    for name in names {
        let text = name.to_string_lossy();
        if text.starts_with("directory-") {
            directories += 1;
            continue;
        }
        if name == RUN_MARKER
            || name == "quarantine"
            || text.starts_with(".tmp-write-")
            || !text.ends_with(".json")
        {
            continue;
        }
        let bytes = read_plain(&journal, Path::new(&name)).map_err(|error| {
            io::Error::new(error.kind(), format!("cannot read journal record {text}: {error}"))
        })?;
        match Entry::from_wire(&bytes) {
            Ok(entry) => {
                let index = u32::try_from(entries.len()).unwrap_or(u32::MAX);
                entries.push(entry_view(index, &entry, home, committed));
            }
            Err(rejection) => unadmitted.push(format!("{text}: {rejection}")),
        }
    }
    Ok(Some(JournalView {
        pending,
        verdict,
        entries,
        unadmitted,
        directories,
    }))
}
