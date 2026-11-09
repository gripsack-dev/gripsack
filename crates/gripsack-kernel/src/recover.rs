//! The recover machine (plan/0056 §4, F2 step 2): crash recovery of the
//! deploy journal as a sans-IO state machine.
//!
//! The host's driver feeds one [`Event`] at a time and executes the one
//! [`Command`] the machine returns: observe → decide → one command → read its
//! result. The machine never touches the world. What it decides:
//!
//! - **commit classification** of the interrupted run, by the proved
//!   [`classify`] (K3) over the run marker and the live `current` pointer;
//! - **per entry** (uncommitted runs only), from the entry's typed identities
//!   and the observed live object ([`decide`]): restore the prior, seal an
//!   observed prior, preserve a foreign edit, or refuse an object whose
//!   identity cannot be established (a directory, a special file, a link with
//!   a non-UTF-8 target) — the unrestorable-entry wedge. A refusal names the
//!   entry and changes nothing: recovery stays fail-closed, user data is
//!   never deleted to make progress;
//! - **the guard**: every restore carries the live identity it was decided
//!   on as `expect`; the driver re-checks it immediately before acting and
//!   answers [`Failure::Drift`] on mismatch, and the restore must land
//!   exactly the prior identity;
//! - **cleanup order**: every journal record unlinked, one journal barrier,
//!   the run marker unlinked, a second barrier (0026 §5). A committed run
//!   first seals the observed selection (a barrier on the directory holding
//!   `current`) before any evidence goes.
//!
//! Directory records (journal v3) stay with the legacy directory domain: the
//! machine only schedules the host's delegated steps ([`Delegated`]) at the
//! points the legacy recovery ran them, and skips entries the host reports as
//! settled by that domain ([`Route::Delegated`]).
//!
//! Proved (proofs/Kernel/Recover.lean): a restore is issued only for the
//! entry under decision, with `expect` equal to the observed live object, and
//! only when that object is the run's own write (intended or immediate
//! pre-state) or absent with a prior to bring back; an unrestorable object
//! yields a refusal naming the entry, never a mutation; the marker unlink is
//! issued only right after a journal barrier that follows the unlink of every
//! record; and a restore is issued only in a run the classification declared
//! uncommitted (or a legacy run with no marker), after `current` was read.
//!
//! Aeneas shape (lib.rs): no derives (impls in `convenience.rs`), equality as
//! explicit `same` functions, owned `Copy` facts, no casts, no closures.
//! Counters advance with `checked_add` (a pure `Option` in the translation)
//! so every step is total: an overflow is a refusal, never a failed step.
use crate::ids::{DestId, Digest};
use crate::selection::{Classification, RecoveryFacts, SelectionIdentity, classify};

/// A leaf object's identity as the host computed it from one observation:
/// a regular file (content and full mode, digested) or a symlink (its target
/// bytes, digested). A file never equals a link, whatever the digests.
pub enum Ident {
    File(Digest),
    Link(Digest),
}

impl Ident {
    pub fn same(&self, other: &Ident) -> bool {
        match (self, other) {
            (Ident::File(a), Ident::File(b)) => a.same(b),
            (Ident::Link(a), Ident::Link(b)) => a.same(b),
            _ => false,
        }
    }
}

/// What a destination name holds right now (one `lstat` plus, for a file or
/// a link, its content). `Unsupported` is anything whose identity the journal
/// cannot express: a directory, a FIFO, a socket, a device, a symlink whose
/// target is not UTF-8.
pub enum Live {
    Absent,
    Obj(Ident),
    Unsupported,
}

impl Live {
    pub fn same(&self, other: &Live) -> bool {
        match (self, other) {
            (Live::Absent, Live::Absent) => true,
            (Live::Obj(a), Live::Obj(b)) => a.same(b),
            (Live::Unsupported, Live::Unsupported) => true,
            _ => false,
        }
    }
}

/// The state an entry restores: the run-original prior.
pub enum Prior {
    Absent,
    Obj(Ident),
}

impl Prior {
    /// Whether `live` is exactly this prior.
    pub fn matches(&self, live: &Live) -> bool {
        match (self, live) {
            (Prior::Absent, Live::Absent) => true,
            (Prior::Obj(a), Live::Obj(b)) => a.same(b),
            _ => false,
        }
    }
}

/// The post-state the journaled mutation intended to create.
pub enum Intended {
    Removed,
    Obj(Ident),
}

/// Who settles an entry: this machine, or the legacy directory domain (the
/// entry lies inside a directory transition that domain already recovered).
pub enum Route {
    Leaf,
    Delegated,
}

/// One admitted journal entry, as the host decoded it. `before` is the
/// immediate pre-state of a repeated write (journal v2) when it differs from
/// the prior; `None` otherwise.
pub struct EntryFacts {
    pub dest: DestId,
    pub prior: Prior,
    pub before: Option<Ident>,
    pub intended: Intended,
    pub route: Route,
}

/// The run marker's declared transaction.
pub struct MarkerFacts {
    pub previous: Option<SelectionIdentity>,
    pub target: SelectionIdentity,
}

/// The admitted journal: entry and directory-record counts, how many leading
/// entries are reverse directory roots (the host orders them first), and the
/// marker if one is present.
pub struct JournalFacts {
    pub entries: u32,
    pub directories: u32,
    pub reverse_roots: u32,
    pub marker: Option<MarkerFacts>,
}

/// The per-entry decision (0026 §6 plus the unrestorable refusal).
pub enum Decision {
    /// The live object is the run's own write (or absence where a prior
    /// existed): bring the prior back.
    Restore,
    /// The prior is what is live; seal it durable before cleanup.
    Unchanged,
    /// A foreign object replaced the run's write: it stands, untouched.
    Preserve,
    /// The live object's identity cannot be established: stop, change
    /// nothing, name the entry.
    Refuse,
}

/// Decide one entry against the observed live object. Ordered exactly as
/// the legacy `decide_from`: the intended write first, then the prior, then
/// the immediate pre-state.
pub fn decide(entry: &EntryFacts, live: &Live) -> Decision {
    match live {
        Live::Unsupported => Decision::Refuse,
        Live::Absent => match entry.prior {
            Prior::Obj(_) => Decision::Restore,
            Prior::Absent => Decision::Unchanged,
        },
        Live::Obj(object) => {
            if intends(&entry.intended, object) {
                Decision::Restore
            } else if entry.prior.matches(live) {
                Decision::Unchanged
            } else if precedes(&entry.before, object) {
                Decision::Restore
            } else {
                Decision::Preserve
            }
        }
    }
}

/// Whether `object` is the intended post-state (shared with the ownership
/// decision).
pub(crate) fn intends(intended: &Intended, object: &Ident) -> bool {
    match intended {
        Intended::Removed => false,
        Intended::Obj(ident) => ident.same(object),
    }
}

fn precedes(before: &Option<Ident>, object: &Ident) -> bool {
    match before {
        None => false,
        Some(ident) => ident.same(object),
    }
}

/// A journal file the cleanup removes: record `k` (entries first, then
/// directory records, in the host's order) or the run marker.
pub enum JournalName {
    Record(u32),
    Marker,
}

/// Which directory a barrier makes durable.
pub enum Scope {
    /// The journal directory.
    Journal,
    /// The directory holding the `current` selection pointer.
    Selection,
}

/// Steps of the legacy directory domain, run by the host when scheduled.
pub enum Delegated {
    /// Committed run: seal staged directory transitions.
    Committed,
    /// Uncommitted run: directory recovery before the first non-root leaf.
    BeforeLeaves,
    /// Uncommitted run: directory recovery after every leaf.
    AfterLeaves,
}

/// The only mutation the recover machine issues.
pub enum Effect {
    /// Put the prior back (remove, rewrite bytes and mode, or re-link).
    Restore(Prior),
}

pub enum Outcome {
    /// No journal directory: nothing was interrupted.
    NoJournal,
    /// The interrupted run had committed: evidence cleared, content stands.
    Committed,
    /// The interrupted run was rolled back to its priors.
    Recovered,
}

/// Why recovery stopped. Every refusal leaves the journal intact.
pub enum Refusal {
    /// `current` matches neither the marker's target nor its predecessor.
    Ambiguous,
    /// Directory records exist without a run marker.
    MarkerMissing,
    /// The journal's counts are inconsistent.
    Malformed,
    /// The entry's live object has no journal identity (the wedge).
    Unrestorable { entry: u32, dest: DestId },
    /// The live object changed between the decision and the act.
    Drifted { entry: u32, dest: DestId },
    /// The restore did not land the prior identity.
    NotRestored { entry: u32, dest: DestId },
    /// The host failed to execute a command.
    Failed,
    /// An event that does not answer the outstanding command.
    Protocol,
}

pub enum Command {
    ReadJournal,
    ReadCurrent,
    ReadEntry(u32),
    Observe {
        entry: u32,
        dest: DestId,
    },
    /// Re-check `expect` against the live object immediately before acting;
    /// answer `Failed(Drift)` on mismatch, else act and answer `Landed`.
    Mutate {
        entry: u32,
        dest: DestId,
        expect: Live,
        effect: Effect,
    },
    /// Make the observed prior durable (file barrier for a regular file, then
    /// the parent's barrier), after re-checking `expect`.
    Seal {
        entry: u32,
        dest: DestId,
        expect: Live,
        prior: Prior,
    },
    /// Leave a foreign object in place (reported as a warning).
    Preserve {
        entry: u32,
        dest: DestId,
    },
    Delegate(Delegated),
    Unlink(JournalName),
    Barrier(Scope),
    Finish(Outcome),
    Abort(Refusal),
}

pub enum Failure {
    /// The re-check before an act found a different live object.
    Drift,
    /// The command failed with this errno (0 when there is none).
    Io(i32),
}

pub enum Event {
    Start,
    NoJournal,
    JournalRead(JournalFacts),
    CurrentRead(Option<SelectionIdentity>),
    EntryRead(EntryFacts),
    Observed { dest: DestId, live: Live },
    Landed { live: Live },
    Done,
    Failed(Failure),
}

pub enum Phase {
    Start,
    ReadingJournal,
    ReadingCurrent,
    SealingSelection,
    SettlingDirectories,
    OpeningLeaves,
    ReadingEntry,
    Observing,
    Restoring,
    Sealing,
    Preserving,
    ClosingLeaves,
    UnlinkingRecord,
    SyncingRecords,
    UnlinkingMarker,
    SyncingMarker,
    Finished,
    Aborted,
}

/// The commit verdict recovery acts under.
pub enum Verdict {
    Pending,
    Committed,
    Uncommitted,
}

pub struct RecoverState {
    pub phase: Phase,
    pub journal: JournalFacts,
    /// Entries plus directory records: what cleanup unlinks.
    pub records: u32,
    /// The `current` pointer as read (when the journal has a marker).
    pub current: Option<SelectionIdentity>,
    pub verdict: Verdict,
    /// The entry under decision, or the next record to unlink in cleanup.
    pub index: u32,
    /// The directory domain's before-leaves step has been scheduled.
    pub leaves_open: bool,
    pub entry: Option<EntryFacts>,
}

impl RecoverState {
    pub fn new() -> RecoverState {
        RecoverState {
            phase: Phase::Start,
            journal: JournalFacts {
                entries: 0,
                directories: 0,
                reverse_roots: 0,
                marker: None,
            },
            records: 0,
            current: None,
            verdict: Verdict::Pending,
            index: 0,
            leaves_open: false,
            entry: None,
        }
    }
}

fn refuse(mut state: RecoverState, refusal: Refusal) -> (RecoverState, Command) {
    state.phase = Phase::Aborted;
    (state, Command::Abort(refusal))
}

fn failed_or_protocol(state: RecoverState, event: Event) -> (RecoverState, Command) {
    match event {
        Event::Failed(_) => refuse(state, Refusal::Failed),
        _ => refuse(state, Refusal::Protocol),
    }
}

/// Unlink the next record, or seal the record unlinks once all are issued.
fn next_unlink(mut state: RecoverState) -> (RecoverState, Command) {
    if state.index < state.records {
        state.phase = Phase::UnlinkingRecord;
        let record = state.index;
        (state, Command::Unlink(JournalName::Record(record)))
    } else {
        state.phase = Phase::SyncingRecords;
        (state, Command::Barrier(Scope::Journal))
    }
}

fn begin_cleanup(mut state: RecoverState) -> (RecoverState, Command) {
    state.index = 0;
    state.entry = None;
    next_unlink(state)
}

/// Continue the uncommitted run at `state.index`: the directory domain's
/// before-leaves step at the first non-root entry, the next entry, the
/// after-leaves step, then cleanup.
fn proceed(mut state: RecoverState) -> (RecoverState, Command) {
    let directories = state.journal.directories != 0;
    if directories && !state.leaves_open && state.index == state.journal.reverse_roots {
        state.leaves_open = true;
        state.phase = Phase::OpeningLeaves;
        (state, Command::Delegate(Delegated::BeforeLeaves))
    } else if state.index < state.journal.entries {
        state.phase = Phase::ReadingEntry;
        let entry = state.index;
        (state, Command::ReadEntry(entry))
    } else if directories {
        state.phase = Phase::ClosingLeaves;
        (state, Command::Delegate(Delegated::AfterLeaves))
    } else {
        begin_cleanup(state)
    }
}

/// The entry under decision is settled: move to the next one.
fn advance(mut state: RecoverState) -> (RecoverState, Command) {
    state.entry = None;
    match state.index.checked_add(1) {
        None => refuse(state, Refusal::Malformed),
        Some(next) => {
            state.index = next;
            proceed(state)
        }
    }
}

fn on_journal(mut state: RecoverState, journal: JournalFacts) -> (RecoverState, Command) {
    if journal.reverse_roots > journal.entries {
        return refuse(state, Refusal::Malformed);
    }
    state.records = match journal.entries.checked_add(journal.directories) {
        None => return refuse(state, Refusal::Malformed),
        Some(records) => records,
    };
    state.journal = journal;
    match journal.marker {
        Some(_) => {
            state.phase = Phase::ReadingCurrent;
            (state, Command::ReadCurrent)
        }
        None => {
            if journal.directories != 0 {
                refuse(state, Refusal::MarkerMissing)
            } else {
                // entries without a marker: a legacy uncommitted run
                state.verdict = Verdict::Uncommitted;
                state.index = 0;
                proceed(state)
            }
        }
    }
}

fn on_current(
    mut state: RecoverState,
    current: Option<SelectionIdentity>,
) -> (RecoverState, Command) {
    state.current = current;
    match state.journal.marker {
        None => refuse(state, Refusal::Protocol),
        Some(marker) => {
            let facts = RecoveryFacts {
                previous: marker.previous,
                target: marker.target,
                current,
            };
            match classify(facts) {
                Classification::Committed => {
                    state.verdict = Verdict::Committed;
                    state.phase = Phase::SealingSelection;
                    (state, Command::Barrier(Scope::Selection))
                }
                Classification::Uncommitted => {
                    state.verdict = Verdict::Uncommitted;
                    state.index = 0;
                    proceed(state)
                }
                Classification::Ambiguous => refuse(state, Refusal::Ambiguous),
            }
        }
    }
}

fn on_entry(mut state: RecoverState, facts: EntryFacts) -> (RecoverState, Command) {
    match facts.route {
        Route::Delegated => advance(state),
        Route::Leaf => {
            let entry = state.index;
            let dest = facts.dest;
            state.entry = Some(facts);
            state.phase = Phase::Observing;
            (state, Command::Observe { entry, dest })
        }
    }
}

fn on_observed(mut state: RecoverState, dest: DestId, live: Live) -> (RecoverState, Command) {
    let facts = match state.entry {
        None => return refuse(state, Refusal::Protocol),
        Some(facts) => facts,
    };
    if !facts.dest.same(dest) {
        return refuse(state, Refusal::Protocol);
    }
    let entry = state.index;
    match decide(&facts, &live) {
        Decision::Restore => {
            state.phase = Phase::Restoring;
            (
                state,
                Command::Mutate {
                    entry,
                    dest,
                    expect: live,
                    effect: Effect::Restore(facts.prior),
                },
            )
        }
        Decision::Unchanged => {
            state.phase = Phase::Sealing;
            (
                state,
                Command::Seal {
                    entry,
                    dest,
                    expect: live,
                    prior: facts.prior,
                },
            )
        }
        Decision::Preserve => {
            state.phase = Phase::Preserving;
            (state, Command::Preserve { entry, dest })
        }
        Decision::Refuse => refuse(state, Refusal::Unrestorable { entry, dest }),
    }
}

fn on_landed(state: RecoverState, live: Live) -> (RecoverState, Command) {
    match state.entry {
        None => refuse(state, Refusal::Protocol),
        Some(facts) => {
            if facts.prior.matches(&live) {
                advance(state)
            } else {
                let entry = state.index;
                refuse(
                    state,
                    Refusal::NotRestored {
                        entry,
                        dest: facts.dest,
                    },
                )
            }
        }
    }
}

fn on_restore_failed(state: RecoverState, failure: Failure) -> (RecoverState, Command) {
    match failure {
        Failure::Io(_) => refuse(state, Refusal::Failed),
        Failure::Drift => match state.entry {
            None => refuse(state, Refusal::Protocol),
            Some(facts) => {
                let entry = state.index;
                refuse(
                    state,
                    Refusal::Drifted {
                        entry,
                        dest: facts.dest,
                    },
                )
            }
        },
    }
}

fn finish(mut state: RecoverState) -> (RecoverState, Command) {
    state.phase = Phase::Finished;
    match state.verdict {
        Verdict::Committed => (state, Command::Finish(Outcome::Committed)),
        _ => (state, Command::Finish(Outcome::Recovered)),
    }
}

/// One step: consume the answer to the previous command (or `Start`) and
/// return the next command. Any event that does not answer the outstanding
/// command aborts with [`Refusal::Protocol`]; a failed command aborts with
/// [`Refusal::Failed`] (or [`Refusal::Drifted`] for a restore's re-check).
pub fn recover_step(state: RecoverState, event: Event) -> (RecoverState, Command) {
    match state.phase {
        Phase::Start => match event {
            Event::Start => {
                let mut next = state;
                next.phase = Phase::ReadingJournal;
                (next, Command::ReadJournal)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingJournal => match event {
            Event::NoJournal => {
                let mut next = state;
                next.phase = Phase::Finished;
                (next, Command::Finish(Outcome::NoJournal))
            }
            Event::JournalRead(journal) => on_journal(state, journal),
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingCurrent => match event {
            Event::CurrentRead(current) => on_current(state, current),
            _ => failed_or_protocol(state, event),
        },
        Phase::SealingSelection => match event {
            Event::Done => {
                if state.journal.directories != 0 {
                    let mut next = state;
                    next.phase = Phase::SettlingDirectories;
                    (next, Command::Delegate(Delegated::Committed))
                } else {
                    begin_cleanup(state)
                }
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::SettlingDirectories | Phase::ClosingLeaves => match event {
            Event::Done => begin_cleanup(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::OpeningLeaves => match event {
            Event::Done => proceed(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingEntry => match event {
            Event::EntryRead(facts) => on_entry(state, facts),
            _ => failed_or_protocol(state, event),
        },
        Phase::Observing => match event {
            Event::Observed { dest, live } => on_observed(state, dest, live),
            _ => failed_or_protocol(state, event),
        },
        Phase::Restoring => match event {
            Event::Landed { live } => on_landed(state, live),
            Event::Failed(failure) => on_restore_failed(state, failure),
            _ => refuse(state, Refusal::Protocol),
        },
        Phase::Sealing => match event {
            Event::Done => advance(state),
            Event::Failed(failure) => on_restore_failed(state, failure),
            _ => refuse(state, Refusal::Protocol),
        },
        Phase::Preserving => match event {
            Event::Done => advance(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::UnlinkingRecord => match event {
            Event::Done => match state.index.checked_add(1) {
                None => refuse(state, Refusal::Malformed),
                Some(index) => {
                    let mut next = state;
                    next.index = index;
                    next_unlink(next)
                }
            },
            _ => failed_or_protocol(state, event),
        },
        Phase::SyncingRecords => match event {
            Event::Done => {
                let mut next = state;
                next.phase = Phase::UnlinkingMarker;
                (next, Command::Unlink(JournalName::Marker))
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::UnlinkingMarker => match event {
            Event::Done => {
                let mut next = state;
                next.phase = Phase::SyncingMarker;
                (next, Command::Barrier(Scope::Journal))
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::SyncingMarker => match event {
            Event::Done => finish(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Finished | Phase::Aborted => refuse(state, Refusal::Protocol),
    }
}
