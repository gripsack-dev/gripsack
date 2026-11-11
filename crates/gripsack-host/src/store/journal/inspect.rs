//! A read-only view of a pending journal, for `grip doctor` (0058 item 1).
//!
//! Recovery runs before apply, rollback and gc; when it refuses, the user
//! needs to see *which* entry blocks it, what recovery expected there, what
//! it found, and what to do. This view reads the same evidence recovery
//! reads and asks the kernel's own per-entry [`decide`] — the function the
//! recover machine uses — but changes nothing: no quarantine moves, no
//! straggler cleanup, no permission repair, no directory creation.
use super::recover_kernel::{entry_facts, expectation, file_ident, link_ident, prior_facts};
use super::storage::{Journal, RUN_MARKER};
use super::{Entry, PendingRecovery, pending_recovery};
use crate::fs::Dir;
use crate::recover::{Found, observe_with};
use crate::storage::{Name, RealFs};
use gripsack_kernel::recover::{Decision, decide};
use gripsack_kernel::selection::{RecoveryFacts, classify};
use gripsack_kernel::{Classification, selection::SelectionIdentity};
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
    Decided(Decision),
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
    pub found: Found,
    pub outlook: EntryOutlook,
}

impl EntryView {
    /// Recovery refuses this entry: its live object has no journal identity.
    pub fn blocks(&self) -> bool {
        matches!(self.outlook, EntryOutlook::Decided(Decision::Refuse))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalView {
    pub pending: PendingRecovery,
    /// `None` when the marker could not be read or classified.
    pub verdict: Option<RunVerdict>,
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

fn verdict(home: &Dir, home_path: &Path, journal: &Journal) -> Option<RunVerdict> {
    let marker = match read_plain(journal, Path::new(RUN_MARKER)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Some(RunVerdict::Unmarked),
        Err(_) => return None,
        Ok(bytes) => serde_json::from_slice::<super::marker::RunMarker>(&bytes).ok()?,
    };
    let current: Option<SelectionIdentity> =
        crate::store::generations::current_selection_in(home_path, home).ok()?;
    Some(
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

/// Observe and decide one entry without effects.
fn entry_view(index: u32, entry: &Entry, home: &Dir, committed: bool) -> EntryView {
    let expected = expectation(entry);
    let dest = Path::new(&entry.dest);
    let observed = (|| -> io::Result<(gripsack_kernel::recover::Live, Found)> {
        let parent = dest.parent().unwrap_or_else(|| Path::new("."));
        let parent = match crate::fs::open(parent) {
            Ok(parent) => parent,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok((gripsack_kernel::recover::Live::Absent, Found::Absent));
            }
            Err(error) => return Err(error),
        };
        let leaf = Name::new(dest.file_name().unwrap_or_default())?;
        let storage = RealFs::new("/");
        observe_with(
            &storage,
            &super::recover_kernel::real_dir(&parent)?,
            &leaf,
            file_ident,
            link_ident,
        )
    })();
    let (live, found) = match observed {
        Ok(observed) => observed,
        Err(error) => {
            return EntryView {
                dest: entry.dest.clone(),
                expected,
                found: Found::Special,
                outlook: EntryOutlook::Unreadable(error.to_string()),
            };
        }
    };
    let outlook = if committed {
        EntryOutlook::Committed
    } else {
        match prior_facts(&entry.prior, home)
            .and_then(|(prior, _)| entry_facts(index, entry, prior.as_ref(), home))
        {
            Ok(facts) => EntryOutlook::Decided(decide(&facts, &live)),
            Err(error) => EntryOutlook::Unreadable(error.to_string()),
        }
    };
    EntryView {
        dest: entry.dest.clone(),
        expected,
        found,
        outlook,
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
    let verdict = verdict(home, home_path, &journal);
    let committed = verdict == Some(RunVerdict::Committed);
    let mut entries = Vec::new();
    let mut unadmitted = Vec::new();
    let mut directories = 0;
    let mut names: Vec<_> = journal
        .directory
        .read_dir(".")?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
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
        match read_plain(&journal, Path::new(&name)).map(|bytes| Entry::from_wire(&bytes)) {
            Ok(Ok(entry)) => {
                let index = u32::try_from(entries.len()).unwrap_or(u32::MAX);
                entries.push(entry_view(index, &entry, home, committed));
            }
            Ok(Err(rejection)) => unadmitted.push(format!("{text}: {rejection}")),
            Err(error) => unadmitted.push(format!("{text}: {error}")),
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
