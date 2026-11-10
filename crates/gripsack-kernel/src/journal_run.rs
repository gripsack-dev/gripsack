//! The journal run (plan/0056 §4, F2 step 3): one transaction's journal, and
//! the leaf effect protocol for each destination it mutates, as a sans-IO
//! state machine.
//!
//! The host's driver feeds one [`Event`] at a time and executes the one
//! [`Command`] the machine returns. The machine never touches the world.
//! What it decides:
//!
//! - **the run marker** is published first, through the publication order of
//!   [`PublicationAction`] (stage, sync the file, publish the name, sync the
//!   journal directory), before the machine accepts any unit;
//! - **the leaf protocol**, per destination: observe it; refuse unless it is
//!   exactly what the plan observed (and, for a repeated write, what this
//!   run's earlier entry left there); capture the prior durably under a
//!   re-check of that observation; publish the entry (the same four steps);
//!   only then mutate, under a re-check of the same observation; and accept
//!   the unit only when the landed object is the intended one;
//! - **completion**: a committed run flips the selection first and removes
//!   journal evidence only after the flip is acknowledged durable; a run that
//!   ends without committing may remove its marker only if it never journaled
//!   a destination. Cleanup unlinks every record, makes that durable, then
//!   unlinks the marker and makes that durable (0026 §5).
//!
//! Directory records stay with the legacy directory domain: the host settles
//! them when told to ([`Command::Settle`]) and reports how many journal
//! records the cleanup removes.
//!
//! Proved (proofs/Kernel/JournalRun.lean, P1 and part of P3): in every
//! execution, a mutation is issued only right after this sequence for its
//! destination, each step answered `Done` — observe; capture the prior with
//! `expect` equal to that observation, which is the plan's; stage, sync,
//! publish and sync the entry — and after the run marker's namespace barrier
//! was acknowledged. A journal record is unlinked only after the selection
//! flip was acknowledged, or in a run that never journaled a destination;
//! the marker unlink only right after a barrier that follows every record
//! unlink; and a unit is accepted only when it landed its intended object.
//!
//! Aeneas shape (lib.rs): no derives (impls in `convenience.rs`), equality as
//! explicit `same` functions, owned `Copy` facts, no casts, no closures;
//! counters advance with `checked_add`, an overflow is a refusal.
use crate::ids::DestId;
use crate::journal::PublicationAction;
use crate::recover::{Failure, Intended, JournalName, Live};
use crate::rollback::lands;

/// One destination the run is about to mutate, as the host planned it.
pub struct UnitFacts {
    pub dest: DestId,
    /// What the plan observed: the act must find exactly this.
    pub expect: Live,
    pub intended: Intended,
    /// A repeated write in this run: the post-state this run's entry for the
    /// destination already intends. The destination must still hold it.
    pub follows: Option<Intended>,
}

/// Which journal record a publication step writes.
pub enum Record {
    Marker,
    /// The entry of the unit under way.
    Entry,
}

/// What the host settles before cleanup (the legacy directory domain).
pub enum Settle {
    /// The flip is durable: seal staged directory transitions.
    Committed,
    /// The run ends without a flip.
    Ended,
}

pub enum Outcome {
    Committed,
    Ended,
}

/// Why the run stopped. The journal is retained; recovery compensates.
pub enum Refusal {
    /// The destination is not what the plan observed (at the observation or
    /// at a re-check before an act).
    Drifted { unit: u32, dest: DestId },
    /// A repeated write found something other than this run's earlier write.
    Interleaved { unit: u32, dest: DestId },
    /// The destination has no journal identity (a directory, a special
    /// file, a link with a non-UTF-8 target).
    Unsupported { unit: u32, dest: DestId },
    /// The mutation did not land the intended object.
    NotLanded { unit: u32, dest: DestId },
    /// The run asked to end after journaling a destination.
    Unfinished,
    /// The host failed to execute a command.
    Failed,
    /// An event that does not answer the outstanding command.
    Protocol,
    /// A counter would overflow.
    Overflow,
}

pub enum Command {
    /// One step of a record's publication.
    Publish {
        record: Record,
        action: PublicationAction,
    },
    /// The machine waits for the next unit, the commit or the end.
    Ready,
    Observe {
        unit: u32,
        dest: DestId,
    },
    /// Re-check `expect`, then make the prior durable evidence (a regular
    /// file's bytes and mode; a link's target and an absence travel in the
    /// entry). Answer `Failed(Drift)` on mismatch.
    CapturePrior {
        unit: u32,
        dest: DestId,
        expect: Live,
    },
    /// Re-check `expect` immediately before acting; answer `Failed(Drift)`
    /// on mismatch, else act and answer `Landed`.
    Mutate {
        unit: u32,
        dest: DestId,
        expect: Live,
        intended: Intended,
    },
    /// Make the reserved selection current, durably.
    Flip,
    Settle(Settle),
    Unlink(JournalName),
    /// The journal directory's namespace barrier.
    Barrier,
    Finish(Outcome),
    Abort(Refusal),
}

pub enum Event {
    Start,
    Done,
    Failed(Failure),
    /// The next destination to mutate (answers `Ready`).
    Unit(UnitFacts),
    /// The run's units are done; flip (answers `Ready`).
    Commit,
    /// The run ends without a flip (answers `Ready`).
    End,
    Observed {
        dest: DestId,
        live: Live,
    },
    Landed {
        live: Live,
    },
    /// The host settled its domain; cleanup removes this many records.
    Settled {
        records: u32,
    },
}

pub enum Phase {
    Start,
    PublishingMarker,
    Ready,
    Observing,
    Capturing,
    PublishingEntry,
    Mutating,
    Flipping,
    Settling,
    UnlinkingRecord,
    SyncingRecords,
    UnlinkingMarker,
    SyncingMarker,
    Finished,
    Aborted,
}

pub struct RunState {
    pub phase: Phase,
    /// The publication step in flight (the marker's, then each entry's).
    pub step: PublicationAction,
    /// The flip was acknowledged durable.
    pub committed: bool,
    /// Units that landed; the unit under way has this number.
    pub units: u32,
    pub unit: Option<UnitFacts>,
    /// The unit's observed destination (equal to its `expect`).
    pub observed: Live,
    /// Records the cleanup removes.
    pub records: u32,
    /// The next record to unlink.
    pub index: u32,
}

impl RunState {
    pub fn new() -> RunState {
        RunState {
            phase: Phase::Start,
            step: PublicationAction::Stage,
            committed: false,
            units: 0,
            unit: None,
            observed: Live::Absent,
            records: 0,
            index: 0,
        }
    }
}

fn refuse(mut state: RunState, refusal: Refusal) -> (RunState, Command) {
    state.phase = Phase::Aborted;
    (state, Command::Abort(refusal))
}

fn failed_or_protocol(state: RunState, event: Event) -> (RunState, Command) {
    match event {
        Event::Failed(_) => refuse(state, Refusal::Failed),
        _ => refuse(state, Refusal::Protocol),
    }
}

/// The marker's publication step was acknowledged: the next step, or ready
/// once the namespace barrier completed it.
fn on_marker_step(mut state: RunState) -> (RunState, Command) {
    let next = state.step.successor();
    match next {
        PublicationAction::Complete => {
            state.phase = Phase::Ready;
            (state, Command::Ready)
        }
        _ => {
            state.step = next;
            (
                state,
                Command::Publish {
                    record: Record::Marker,
                    action: next,
                },
            )
        }
    }
}

fn on_unit(mut state: RunState, facts: UnitFacts) -> (RunState, Command) {
    let unit = state.units;
    let dest = facts.dest;
    state.unit = Some(facts);
    state.observed = Live::Absent;
    state.phase = Phase::Observing;
    (state, Command::Observe { unit, dest })
}

/// A repeated write must find what this run's earlier entry intended.
fn follows(previous: &Option<Intended>, live: &Live) -> bool {
    match previous {
        None => true,
        Some(intended) => lands(intended, live),
    }
}

fn on_observed(mut state: RunState, dest: DestId, live: Live) -> (RunState, Command) {
    let facts = match state.unit {
        None => return refuse(state, Refusal::Protocol),
        Some(facts) => facts,
    };
    if !facts.dest.same(dest) {
        return refuse(state, Refusal::Protocol);
    }
    let unit = state.units;
    match live {
        Live::Unsupported => refuse(state, Refusal::Unsupported { unit, dest }),
        _ => {
            if !live.same(&facts.expect) {
                refuse(state, Refusal::Drifted { unit, dest })
            } else if !follows(&facts.follows, &live) {
                refuse(state, Refusal::Interleaved { unit, dest })
            } else {
                state.observed = live;
                state.phase = Phase::Capturing;
                (
                    state,
                    Command::CapturePrior {
                        unit,
                        dest,
                        expect: live,
                    },
                )
            }
        }
    }
}

/// A re-check or an act failed.
fn on_act_failed(state: RunState, failure: Failure) -> (RunState, Command) {
    match failure {
        Failure::Io(_) => refuse(state, Refusal::Failed),
        Failure::Drift => match state.unit {
            None => refuse(state, Refusal::Protocol),
            Some(facts) => {
                let unit = state.units;
                refuse(
                    state,
                    Refusal::Drifted {
                        unit,
                        dest: facts.dest,
                    },
                )
            }
        },
    }
}

fn on_captured(mut state: RunState) -> (RunState, Command) {
    state.step = PublicationAction::Stage;
    state.phase = Phase::PublishingEntry;
    (
        state,
        Command::Publish {
            record: Record::Entry,
            action: PublicationAction::Stage,
        },
    )
}

/// The entry's publication step was acknowledged: the next step, or the
/// mutation once the namespace barrier made the entry durable.
fn on_entry_step(mut state: RunState) -> (RunState, Command) {
    let facts = match state.unit {
        None => return refuse(state, Refusal::Protocol),
        Some(facts) => facts,
    };
    let next = state.step.successor();
    match next {
        PublicationAction::Complete => {
            let unit = state.units;
            let expect = state.observed;
            state.phase = Phase::Mutating;
            (
                state,
                Command::Mutate {
                    unit,
                    dest: facts.dest,
                    expect,
                    intended: facts.intended,
                },
            )
        }
        _ => {
            state.step = next;
            (
                state,
                Command::Publish {
                    record: Record::Entry,
                    action: next,
                },
            )
        }
    }
}

fn on_landed(mut state: RunState, live: Live) -> (RunState, Command) {
    let facts = match state.unit {
        None => return refuse(state, Refusal::Protocol),
        Some(facts) => facts,
    };
    let unit = state.units;
    if !lands(&facts.intended, &live) {
        return refuse(
            state,
            Refusal::NotLanded {
                unit,
                dest: facts.dest,
            },
        );
    }
    match unit.checked_add(1) {
        None => refuse(state, Refusal::Overflow),
        Some(units) => {
            state.units = units;
            state.unit = None;
            state.phase = Phase::Ready;
            (state, Command::Ready)
        }
    }
}

fn on_end(mut state: RunState) -> (RunState, Command) {
    if state.units != 0 {
        return refuse(state, Refusal::Unfinished);
    }
    state.phase = Phase::Settling;
    (state, Command::Settle(Settle::Ended))
}

/// Unlink the next record, or seal the record unlinks once all are issued.
fn next_unlink(mut state: RunState) -> (RunState, Command) {
    if state.index < state.records {
        state.phase = Phase::UnlinkingRecord;
        let record = state.index;
        (state, Command::Unlink(JournalName::Record(record)))
    } else {
        state.phase = Phase::SyncingRecords;
        (state, Command::Barrier)
    }
}

fn finish(mut state: RunState) -> (RunState, Command) {
    state.phase = Phase::Finished;
    if state.committed {
        (state, Command::Finish(Outcome::Committed))
    } else {
        (state, Command::Finish(Outcome::Ended))
    }
}

/// One step: consume the answer to the previous command (or `Start`) and
/// return the next command. Any event that does not answer the outstanding
/// command aborts with [`Refusal::Protocol`]; a failed command aborts with
/// [`Refusal::Failed`] (or [`Refusal::Drifted`] for a failed re-check).
pub fn run_step(state: RunState, event: Event) -> (RunState, Command) {
    match state.phase {
        Phase::Start => match event {
            Event::Start => {
                let mut next = state;
                next.step = PublicationAction::Stage;
                next.phase = Phase::PublishingMarker;
                (
                    next,
                    Command::Publish {
                        record: Record::Marker,
                        action: PublicationAction::Stage,
                    },
                )
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::PublishingMarker => match event {
            Event::Done => on_marker_step(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Ready => match event {
            Event::Unit(facts) => on_unit(state, facts),
            Event::Commit => {
                let mut next = state;
                next.phase = Phase::Flipping;
                (next, Command::Flip)
            }
            Event::End => on_end(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Observing => match event {
            Event::Observed { dest, live } => on_observed(state, dest, live),
            _ => failed_or_protocol(state, event),
        },
        Phase::Capturing => match event {
            Event::Done => on_captured(state),
            Event::Failed(failure) => on_act_failed(state, failure),
            _ => refuse(state, Refusal::Protocol),
        },
        Phase::PublishingEntry => match event {
            Event::Done => on_entry_step(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Mutating => match event {
            Event::Landed { live } => on_landed(state, live),
            Event::Failed(failure) => on_act_failed(state, failure),
            _ => refuse(state, Refusal::Protocol),
        },
        Phase::Flipping => match event {
            Event::Done => {
                let mut next = state;
                next.committed = true;
                next.phase = Phase::Settling;
                (next, Command::Settle(Settle::Committed))
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::Settling => match event {
            Event::Settled { records } => {
                let mut next = state;
                next.records = records;
                next.index = 0;
                next_unlink(next)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::UnlinkingRecord => match event {
            Event::Done => match state.index.checked_add(1) {
                None => refuse(state, Refusal::Overflow),
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
                (next, Command::Barrier)
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
