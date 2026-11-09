//! Reconcile through the kernel's recover machine (plan/0056 F2 step 2).
//!
//! `gripsack_kernel::recover` decides — commit classification, the
//! per-entry restore/seal/keep/refuse decision, the cleanup order — and
//! `gripsack_host::recover` drives it over `RealFs`, re-checking every act
//! against the identity the kernel decided on. This module is the
//! [`RecoverHost`]: the journal wire and its admission (quarantine,
//! stragglers, marker admission, sorting), the current-pointer reader, the
//! destination resolution legacy recovery used (`dest_capability`, or the
//! directory-bound parent), identity hashing, and the legacy directory
//! domain, which still runs as delegated steps.
//!
//! Behaviour matches `reconcile_legacy` except for the unrestorable-entry
//! wedge (0058 item 1): a destination that is now a directory, a special
//! file or a non-UTF-8 symlink is refused by name with what was expected,
//! what was found and how to repair it, instead of failing on an
//! unexplained observation error. The journal is retained either way.
use super::recover::{NoteSeverity, RecoveryNote};
use super::storage::{Journal, RUN_MARKER};
use super::{Entry, Intended, ObjectIdentity, PriorSerde, read_uncommitted};
use gripsack_fs::Dir;
use gripsack_host::recover::{
    Found, HostEntry, LeafSite, RecoverError, RecoverHost, RestoreSource, Settled, recover,
};
use gripsack_host::storage::{Name, Permissions, RealDir, RealFs};
use gripsack_kernel::recover::{
    Delegated, EntryFacts, Ident, Intended as KernelIntended, JournalFacts, MarkerFacts, Outcome,
    Prior, Refusal, Route,
};
use gripsack_kernel::selection::SelectionIdentity;
use gripsack_kernel::{DestId, Digest};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The same pinned directory as a `RealFs` handle (same inode).
pub(super) fn real_dir(dir: &Dir) -> io::Result<RealDir> {
    use std::os::fd::AsFd;
    RealDir::pin(dir.as_fd())
}

fn name(path: &Path) -> io::Result<Name> {
    Name::new(path.as_os_str()).map_err(io::Error::from)
}

fn digest_hex(hex: &str) -> io::Result<Digest> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "malformed identity digest");
    if hex.len() != 64 {
        return Err(invalid());
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * index..2 * index + 2], 16).map_err(|_| invalid())?;
    }
    Ok(Digest::from_bytes(bytes))
}

/// The journal's typed identity as a kernel identity. Links compare their
/// target verbatim in the journal; the kernel compares SHA-256 digests of
/// the target bytes (S7).
pub(super) fn kernel_ident(identity: &ObjectIdentity) -> io::Result<Ident> {
    Ok(match identity {
        ObjectIdentity::File(file) => Ident::File(digest_hex(file.as_str())?),
        ObjectIdentity::Link(target) => Ident::Link(link_digest(target.as_bytes())),
    })
}

fn link_digest(target: &[u8]) -> Digest {
    digest_hex(&crate::hash::hex_sha256(target)).expect("sha256 hex is 64 digits")
}

/// What restoring this prior puts back, and the prior's typed identity.
pub(super) fn prior_facts(
    prior: &PriorSerde,
    home: &Dir,
) -> io::Result<(Option<ObjectIdentity>, RestoreSource)> {
    Ok(match prior {
        PriorSerde::Absent => (None, RestoreSource::Absent),
        PriorSerde::Symlink { target } => (
            Some(ObjectIdentity::Link(target.clone())),
            RestoreSource::Link {
                target: PathBuf::from(target),
            },
        ),
        PriorSerde::File { hash, mode } => {
            let bytes = crate::prior::read_blob(home, hash)?;
            (
                Some(ObjectIdentity::File(crate::hash::canonical_bytes_identity(
                    &bytes,
                    mode.bits(),
                ))),
                RestoreSource::File {
                    bytes,
                    mode: Permissions::new(mode.bits()),
                },
            )
        }
    })
}

/// The kernel's view of one entry (routed to the leaf machine).
pub(super) fn entry_facts(
    index: u32,
    entry: &Entry,
    prior: Option<&ObjectIdentity>,
    home: &Dir,
) -> io::Result<EntryFacts> {
    let before = if entry.before.as_ref() == Some(&entry.prior) {
        None
    } else {
        match &entry.before {
            None => None,
            Some(before) => super::prior_identity(before, home)?,
        }
    };
    Ok(EntryFacts {
        dest: DestId::new(index),
        prior: match prior {
            None => Prior::Absent,
            Some(identity) => Prior::Obj(kernel_ident(identity)?),
        },
        before: before.as_ref().map(kernel_ident).transpose()?,
        intended: match Intended::from_wire(&entry.after) {
            Intended::Removed => KernelIntended::Removed,
            Intended::Object(identity) => KernelIntended::Obj(kernel_ident(&identity)?),
        },
        route: Route::Leaf,
    })
}

/// File identity in the journal's domain (mode-aware, 0031).
pub(super) fn file_ident(bytes: &[u8], mode: Permissions) -> Ident {
    let identity = crate::hash::canonical_bytes_identity(bytes, mode.bits());
    Ident::File(digest_hex(identity.as_str()).expect("canonical identities are sha256 hex"))
}

/// Link identity; a non-UTF-8 target has none (0045 F1, item 7).
pub(super) fn link_ident(target: &Path) -> Option<Ident> {
    target
        .to_str()
        .map(|target| Ident::Link(link_digest(target.as_bytes())))
}

/// `gripsack_fs`'s temp sibling naming: pid + counter, so a stale one from
/// a crashed process is the only possible collision (and is replaced).
static STAGING: AtomicU64 = AtomicU64::new(0);

struct StoreHost<'a> {
    storage: RealFs,
    home: &'a Dir,
    home_path: &'a Path,
    journal: Option<Journal>,
    entries: Vec<(PathBuf, Entry)>,
    directories: Vec<(PathBuf, super::directory::DirectoryRecord)>,
    marker: Option<super::marker::RunMarker>,
    current: Option<SelectionIdentity>,
    retain_prepared: bool,
}

impl<'a> StoreHost<'a> {
    fn new(home: &'a Dir, home_path: &'a Path) -> Self {
        Self {
            storage: RealFs::new(home_path),
            home,
            home_path,
            journal: None,
            entries: Vec::new(),
            directories: Vec::new(),
            marker: None,
            current: None,
            retain_prepared: false,
        }
    }

    fn journal(&self) -> io::Result<&Journal> {
        self.journal
            .as_ref()
            .ok_or_else(|| io::Error::other("journal not admitted"))
    }

    fn dest(&self, entry: u32) -> String {
        self.entries
            .get(entry as usize)
            .map_or_else(|| format!("journal entry {entry}"), |(_, e)| e.dest.clone())
    }

    /// A refusal as the error the CLI shows.
    fn refusal(&self, refusal: Refusal, found: Option<Found>) -> io::Error {
        let message = match refusal {
            Refusal::Ambiguous => {
                let (previous, target) = self
                    .marker
                    .as_ref()
                    .map(|marker| (marker.previous, Some(marker.target)))
                    .unwrap_or((None, None));
                format!(
                    "journal selections ({previous:?} → {target:?}) match neither current \
                     ({:?}) nor the recorded predecessor — the journal is retained; inspect \
                     $GRIPSACK_HOME/journal before running again",
                    self.current
                )
            }
            Refusal::MarkerMissing => {
                "directory journal lacks its transaction marker; evidence retained".into()
            }
            Refusal::Unrestorable { entry, .. } => {
                let dest = self.dest(entry);
                let expected = self
                    .entries
                    .get(entry as usize)
                    .map(|(_, e)| expectation(e))
                    .unwrap_or_default();
                format!(
                    "recovery cannot restore {dest}: it is now {}, which the journal cannot \
                     identify ({expected}). Nothing was changed and the journal is retained. \
                     Move {dest} aside (it is yours; nothing was deleted), then run the command \
                     again — `grip doctor` shows the pending journal",
                    found.unwrap_or(Found::Special)
                )
            }
            Refusal::Drifted { entry, .. } => format!(
                "{} changed while recovery was restoring it — nothing was overwritten and the \
                 journal is retained; run the command again",
                self.dest(entry)
            ),
            Refusal::NotRestored { entry, .. } => format!(
                "recovery of {} did not produce the prior state — the journal is retained; \
                 inspect $GRIPSACK_HOME/journal",
                self.dest(entry)
            ),
            Refusal::Malformed | Refusal::Failed | Refusal::Protocol => format!(
                "journal recovery stopped ({refusal:?}); the journal is retained — inspect \
                 $GRIPSACK_HOME/journal"
            ),
        };
        io::Error::other(message)
    }
}

/// What recovery would accept at an entry's destination, for people.
pub(super) fn expectation(entry: &Entry) -> String {
    let after = match &entry.after {
        super::IntendedSerde::Removed => "its removal".to_string(),
        super::IntendedSerde::File { .. } => "the file it wrote".to_string(),
        super::IntendedSerde::Link { target } => format!("the symlink → {target} it wrote"),
    };
    let prior = match &entry.prior {
        PriorSerde::Absent => "nothing (absent)".to_string(),
        PriorSerde::File { mode, .. } => format!("the prior file (mode {:04o})", mode.bits()),
        PriorSerde::Symlink { target } => format!("the prior symlink → {target}"),
    };
    format!("the interrupted run left {after}; recovery restores {prior}")
}

impl RecoverHost for StoreHost<'_> {
    type Storage = RealFs;

    fn storage(&self) -> &RealFs {
        &self.storage
    }

    fn read_journal(&mut self) -> io::Result<Option<JournalFacts>> {
        let Some(journal) = Journal::open(self.home)? else {
            return Ok(None);
        };
        let mut entries = read_uncommitted(&journal)?;
        let directories = super::directory::read_records(&journal)?;
        // A rollback can restore the directory's older ownership origin after
        // reinstating its predecessor link. Undo that flat root write first,
        // then restore the exact staged directory, then its child priors.
        entries.sort_by_key(|(_, entry)| {
            (
                !super::directory_recovery::is_reverse_root(&directories, Path::new(&entry.dest)),
                std::cmp::Reverse(Path::new(&entry.dest).components().count()),
            )
        });
        let reverse_roots = entries
            .iter()
            .take_while(|(_, entry)| {
                super::directory_recovery::is_reverse_root(&directories, Path::new(&entry.dest))
            })
            .count();
        // A marker missing `previous_generation` fails closed at parse.
        let marker = super::marker::run_marker(&journal)?;
        if let Some(marker) = &marker {
            marker.admit_recovery()?;
        }
        let count = |n: usize| {
            u32::try_from(n).map_err(|_| io::Error::other("journal too large to recover"))
        };
        let facts = JournalFacts {
            entries: count(entries.len())?,
            directories: count(directories.len())?,
            reverse_roots: count(reverse_roots)?,
            marker: marker.as_ref().map(|marker| MarkerFacts {
                previous: marker.previous,
                target: marker.target,
            }),
        };
        self.journal = Some(journal);
        self.entries = entries;
        self.directories = directories;
        self.marker = marker;
        Ok(Some(facts))
    }

    fn read_current(&mut self) -> io::Result<Option<SelectionIdentity>> {
        // the ONE current-pointer reader (0030 §H10): recovery never uses
        // weaker commit evidence than normal commands
        let current = crate::generations::current_selection_in(self.home_path, self.home)?;
        if let Some(selection) = &current {
            // A matching pointer is not authority over a corrupt or missing
            // generation. Admit the same pinned state before either
            // restoration or committed cleanup can have effects.
            crate::generations::admit_manifest_at(
                self.home,
                self.home_path,
                selection.generation(),
            )?;
        }
        // Directory recovery retains this run's prepared directories only
        // when the target generation's manifest is admissible. (Committed
        // runs have current = target, admitted just above.)
        if !self.directories.is_empty()
            && let Some(marker) = &self.marker
        {
            self.retain_prepared = match crate::generations::admit_manifest_at(
                self.home,
                self.home_path,
                marker.target.generation(),
            ) {
                Ok(_) => true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            };
        }
        self.current = current;
        Ok(current)
    }

    fn read_entry(&mut self, index: u32) -> io::Result<HostEntry<RealDir>> {
        let (_, entry) = self
            .entries
            .get(index as usize)
            .ok_or_else(|| io::Error::other("journal entry index out of range"))?;
        let dest = Path::new(&entry.dest);
        let directories = &self.directories;
        // the directory domain already settled it
        if super::directory_recovery::recovered_reverse_root(directories, dest)?
            || super::directory_recovery::unreachable_leaf(
                directories,
                dest,
                &entry.prior,
                self.home,
                &self.entries,
            )?
        {
            let mut facts = entry_facts(index, entry, None, self.home)?;
            facts.route = Route::Delegated;
            return Ok(HostEntry { facts, site: None });
        }
        // the drift check and the restore share ONE pinned parent inode: a
        // parent symlink swapped between observation and restore cannot
        // redirect the recovery write (plan/0021)
        let (parent, leaf) = match super::directory::recovery_parent(directories, dest)? {
            Some(bound) => bound,
            None => super::dest_capability(dest)?,
        };
        let (prior, restore) = prior_facts(&entry.prior, self.home)?;
        Ok(HostEntry {
            facts: entry_facts(index, entry, prior.as_ref(), self.home)?,
            site: Some(LeafSite {
                parent: real_dir(&parent)?,
                name: name(&leaf)?,
                restore,
            }),
        })
    }

    fn file_ident(&self, bytes: &[u8], mode: Permissions) -> Ident {
        file_ident(bytes, mode)
    }

    fn link_ident(&self, target: &Path) -> Option<Ident> {
        link_ident(target)
    }

    fn staging_name(&self, at: &Name, link: bool) -> Name {
        let prefix = if link { "tmp-link" } else { "tmp-write" };
        Name::new(format!(
            ".{prefix}-{}-{}-{}.tmp",
            std::process::id(),
            STAGING.fetch_add(1, Ordering::Relaxed),
            at.as_os_str().to_string_lossy()
        ))
        .expect("a staging name is one component")
    }

    fn delegate(&mut self, step: Delegated) -> io::Result<()> {
        match step {
            Delegated::Committed => super::directory_lifecycle::committed(&self.directories),
            Delegated::BeforeLeaves => super::directory_recovery::recover_before_leaves(
                &self.directories,
                &self.entries,
                self.home,
                self.retain_prepared,
            ),
            Delegated::AfterLeaves => super::directory_recovery::recover_after_leaves(
                &self.directories,
                self.home,
                self.retain_prepared,
            ),
        }
    }

    fn journal_dir(&self) -> io::Result<RealDir> {
        real_dir(&self.journal()?.directory)
    }

    fn record_name(&self, index: u32) -> io::Result<Name> {
        let index = index as usize;
        let path = match self.entries.get(index) {
            Some((path, _)) => path,
            None => {
                &self
                    .directories
                    .get(index - self.entries.len())
                    .ok_or_else(|| io::Error::other("journal record index out of range"))?
                    .0
            }
        };
        name(path)
    }

    fn marker_name(&self) -> Name {
        Name::new(RUN_MARKER).expect("the marker name is one component")
    }

    fn selection_dir(&self) -> io::Result<RealDir> {
        real_dir(self.home)
    }
}

/// The report text for a restore (presentation stays out of the kernel).
fn describe_prior(prior: &PriorSerde) -> String {
    match prior {
        PriorSerde::Absent => "removed (was absent before the interrupted run)".into(),
        PriorSerde::File { .. } => "prior bytes restored".into(),
        PriorSerde::Symlink { target } => format!("prior symlink → {target} restored"),
    }
}

/// `reconcile` through the kernel machine. Must run under the lifecycle
/// lock.
pub(super) fn reconcile(home: &Dir, home_path: &Path) -> io::Result<Vec<RecoveryNote>> {
    let mut host = StoreHost::new(home, home_path);
    let recovery = match recover(&mut host) {
        Ok(recovery) => recovery,
        Err(RecoverError::Io(error)) => return Err(error),
        Err(RecoverError::Refused { refusal, found }) => {
            return Err(host.refusal(refusal, found));
        }
    };
    if matches!(recovery.outcome, Outcome::Committed) {
        return Ok(vec![RecoveryNote {
            severity: NoteSeverity::Info,
            message: "interrupted run's generation had already activated — journal \
                      cleared, deployed state stands"
                .to_string(),
        }]);
    }
    Ok(recovery
        .notes
        .iter()
        .filter_map(|note| {
            let (_, entry) = host.entries.get(note.entry as usize)?;
            Some(match note.settled {
                Settled::Restored => RecoveryNote {
                    severity: NoteSeverity::Info,
                    message: format!("recovered {}: {}", entry.dest, describe_prior(&entry.prior)),
                },
                Settled::Unchanged => RecoveryNote {
                    severity: NoteSeverity::Info,
                    message: format!("unchanged {}: prior state is durable", entry.dest),
                },
                // a kept post-crash edit is the outcome the user must NOTICE
                Settled::Kept => RecoveryNote {
                    severity: NoteSeverity::Warn,
                    message: format!(
                        "kept {}: changed since the interrupted run — your edit stands",
                        entry.dest
                    ),
                },
            })
        })
        .collect())
}
