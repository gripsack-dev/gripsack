//! The apply machine (plan/0056 §4, F2 step 7): deploying a workspace and
//! cutting its generation as a sans-IO state machine.
//!
//! Apply walks every ownership unit the run touches — first each destination
//! the workspace deploys, then each destination the previous generation
//! deployed that the run may release (prune-on-undeclare) — and settles each
//! one before it publishes and flips. Per unit, from gripsack's record in the
//! previous generation, the unit's intent, the user's consent and one
//! observation of the live object, it decides through the proved ownership
//! decision [`decide_unit`] (K1):
//!
//! - **write** the unit's intended identity into absence or over gripsack's
//!   own last write; under consent (`--take-over`) over anything else, then
//!   installing the identity the host adopts (the intended bytes at the live
//!   object's mode, 0033 R1); or **release** gripsack's own write where the
//!   workspace no longer deploys it (remove it, or bring back the object the
//!   ownership epoch recorded before gripsack took the destination over);
//! - **leave** a destination that already holds the intent, or whose object
//!   is the user's and the unit keeps (`OnDrift::Keep`: a tracked copy's
//!   preserved drift, a prune that finds the destination modified);
//! - **refuse** the whole run before the flip when the unit may not be
//!   touched and must not be kept silently (`OnDrift::Stop`: an owned link
//!   the user replaced; an object without identity in place of a deploy).
//!
//! A release is decided only for a destination no deploy unit of this run
//! claims ([`covered`]), compared by the host's interned destination identity
//! — so a re-spelled destination that the filesystem judges to be the same
//! name is never pruned under the new spelling's feet (FND-3; plan/0056 §4
//! S6). Releases come after every deploy: a deploy unit read after a release
//! aborts the run.
//!
//! Units the host settles itself (managed blocks, K5) are delegated and
//! report whether they changed the disk. The directory domain (step 9) runs
//! as one opaque [`Command::DirectoryStep`] after the units.
//!
//! Then the commit: whether the run cuts a generation at all is the proved
//! [`decide_commit`] (K6), told that the run touched the disk exactly when a
//! unit landed or a delegated unit reported a change (the machine's
//! `mutated` flag, not a report's presentation). A publication is followed
//! by the flip to the reserved selection and the hand-off to activation.
//!
//! Proved (proofs/Kernel/Apply.lean): every write installs exactly the
//! unit's intended identity (the deploy's, the adoption's under consent, or
//! the release's) and answers the observation of the unit under decision
//! (K2); it replaces only absence, gripsack's own last write, or — under
//! consent, for a deploy — anything (K1); a unit is reported applied only
//! when the landed object is that identity; kept and satisfied units never
//! claim it; a release never targets a destination a deploy unit of the run
//! claims (FND-3); the flip comes only after every unit read was reported
//! settled, after the publication, for the reserved selection under a fresh
//! transaction bound to the allocated generation above the floor, and never
//! for a vacuous run; a run that applied any unit never finishes vacuous.
//!
//! Aeneas shape (lib.rs): no derives (impls in `convenience.rs`), equality as
//! explicit `same` functions, owned `Copy` facts, no closures; counters
//! advance with `checked_add`, so every step is total.
use crate::GenerationId;
use crate::ids::DestId;
use crate::ownership::{
    Consent, Decision as UnitDecision, Recorded, Refusal as UnitRefusal, decide_unit,
};
use crate::publication::{Commit, CommitFacts, CommitRefusal, decide_commit};
use crate::recover::{Failure, Ident, Intended, Live, Route};
use crate::rollback::lands;
use crate::selection::SelectionIdentity;
use alloc::vec::Vec;

/// Which part of a destination a unit owns: the whole object, or one managed
/// block in a shared file (its owner, interned by the host).
pub enum Slot {
    Whole,
    Block(u32),
}

/// A destination a deploy unit of this run claims.
pub struct Claim {
    pub dest: DestId,
    pub slot: Slot,
}

/// What the unit wants at its destination.
pub enum Want {
    /// Deploy `ident`. A take-over installs `adopted` instead: the same
    /// content at the mode of the object it absorbs (the host computes it
    /// from the observation the machine decides on).
    Deploy { ident: Ident, adopted: Ident },
    /// The workspace no longer deploys this destination: gripsack's own
    /// write goes, replaced by what the ownership epoch recorded before
    /// gripsack took the destination over (`Obj`), or by nothing (`Removed`).
    Release(Intended),
}

/// What to do when the destination is the user's (drifted or unowned).
pub enum OnDrift {
    /// Leave it and settle the unit (recorded as preserved drift by a copy's
    /// manifest entry; a prune keeps it).
    Keep,
    /// Stop the run before the flip (an owned link the user replaced).
    Stop,
}

/// One unit, as the host prepared it.
pub struct UnitFacts {
    pub dest: DestId,
    pub slot: Slot,
    pub route: Route,
    /// gripsack's record in the previous generation (its lineage).
    pub recorded: Recorded,
    pub want: Want,
    pub consent: Consent,
    pub drift: OnDrift,
}

/// The authority a write carries (the host's report voice follows it).
pub enum Write {
    /// Nothing was live.
    Create,
    /// gripsack's own last write is replaced (or released).
    Replace,
    /// Consent absorbs the live object; the host captures it as the prior.
    TakeOver,
}

/// How a unit was settled (the host reports it).
pub enum Settled {
    /// The intended identity landed.
    Applied,
    /// The destination already held the intent (or its removal).
    Already,
    /// The user's object stands; nothing was written (K1).
    Kept(UnitRefusal),
    /// A release this run's deploys claim: nothing to release.
    Declared,
    /// The host's own domain settled it, changing the disk or not.
    Delegated { mutated: bool },
}

/// The per-unit decision.
pub enum Step {
    /// Install this, with this authority.
    Write { install: Intended, write: Write },
    /// Nothing to write.
    Leave(Settled),
    /// Stop the run before the flip.
    Refuse(Refusal),
}

/// What the end of the run observed, apart from what the machine knows
/// itself (whether it touched the disk).
pub struct CommitInputs {
    /// The selection `current` names: the one the flip replaces.
    pub previous: Option<SelectionIdentity>,
    /// The selection this run reserved for its flip.
    pub reserved: SelectionIdentity,
    /// The generation number this run allocated.
    pub allocated: GenerationId,
    /// The durable high-water floor, read again now.
    pub floor: Option<GenerationId>,
    /// The new manifest's state equals the previous generation's.
    pub unchanged: bool,
    /// A generation directory with the allocated number already exists.
    pub exists: bool,
}

pub enum Outcome {
    /// Nothing changed: no generation, no flip.
    Vacuous,
    /// The generation was published, selected and handed to activation.
    Applied,
}

/// Why apply stopped. Nothing is flipped; the host compensates any write
/// this run made from the journal.
pub enum Refusal {
    /// The unit may not be touched and must not be kept silently.
    Refused {
        unit: u32,
        dest: DestId,
        why: UnitRefusal,
    },
    /// The live object changed between the decision and the act.
    Drifted { unit: u32, dest: DestId },
    /// The write did not land the intended identity.
    NotApplied { unit: u32, dest: DestId },
    /// The host's own domain could not settle a delegated unit.
    Delegated { unit: u32, dest: DestId },
    /// A deploy unit after a release (releases come last).
    Order { unit: u32 },
    /// The generation cannot be published (K6).
    Unpublishable(CommitRefusal),
    /// A counter or list overflowed.
    Malformed,
    /// The host failed to execute a command.
    Failed,
    /// An event that does not answer the outstanding command.
    Protocol,
}

pub enum Command {
    /// Answer `UnitRead` with the next unit, or `Exhausted`.
    ReadUnit(u32),
    Observe {
        unit: u32,
        dest: DestId,
    },
    /// Re-check `expect` against the live object immediately before acting
    /// (answer `Failed(Drift)` on mismatch), install `install` under the
    /// leaf protocol, and answer `Landed` with the observed result.
    Mutate {
        unit: u32,
        dest: DestId,
        expect: Live,
        install: Intended,
        write: Write,
    },
    /// Settle a unit in the host's own domain: answer `Delegated` when
    /// settled, `Refused` when it cannot be.
    Delegate {
        unit: u32,
        dest: DestId,
    },
    /// Record how a unit was settled (the host's progress line); answer
    /// `Done`.
    Report {
        unit: u32,
        dest: DestId,
        settled: Settled,
    },
    /// Run the directory domain's transitions (opaque until step 9); answer
    /// `Done`.
    DirectoryStep,
    /// Answer `CommitRead`.
    ReadCommit,
    /// Publish this generation (and land its pending activation record);
    /// answer `Done` once both are durable.
    Publish(GenerationId),
    /// Point `current` at this selection; answer `Done` once durable.
    Flip(SelectionIdentity),
    /// Hand the new selection to the activation machine; answer `Done`
    /// whatever its hooks returned (a failure is never rolled back).
    Activate,
    Finish(Outcome),
    Abort(Refusal),
}

pub enum Event {
    Start,
    UnitRead(UnitFacts),
    /// No unit is left.
    Exhausted,
    Observed {
        dest: DestId,
        live: Live,
    },
    Landed {
        live: Live,
    },
    /// A delegated unit settled, changing the disk or not.
    Delegated {
        mutated: bool,
    },
    /// A delegated unit cannot be settled.
    Refused,
    Done,
    CommitRead(CommitInputs),
    Failed(Failure),
}

pub enum Phase {
    Start,
    ReadingUnit,
    Observing,
    Writing,
    Delegating,
    Reporting,
    Directories,
    ReadingCommit,
    Publishing,
    Flipping,
    Activating,
    Finished,
    Aborted,
}

pub struct ApplyState {
    pub phase: Phase,
    /// The unit under decision (the number of units read before it).
    pub index: u32,
    pub unit: Option<UnitFacts>,
    /// What the outstanding write installs.
    pub install: Option<Intended>,
    /// Every destination a deploy unit of this run claimed.
    pub declared: Vec<Claim>,
    /// A release was read: no deploy unit may follow.
    pub releasing: bool,
    /// A unit landed, or a delegated unit reported a change.
    pub mutated: bool,
    /// The commit facts a publication was admitted under.
    pub commit: Option<CommitFacts>,
}

impl ApplyState {
    pub fn new() -> ApplyState {
        ApplyState {
            phase: Phase::Start,
            index: 0,
            unit: None,
            install: None,
            declared: Vec::new(),
            releasing: false,
            mutated: false,
            commit: None,
        }
    }
}

/// Whether a deploy's claim covers a release at `dest`: a whole-object
/// release is covered by any deploy at the destination; a block's release
/// only by a deploy of the same block.
pub fn covers(claim: &Claim, dest: DestId, slot: &Slot) -> bool {
    if !claim.dest.same(dest) {
        return false;
    }
    match slot {
        Slot::Whole => true,
        Slot::Block(owner) => match claim.slot {
            Slot::Whole => false,
            Slot::Block(other) => other == *owner,
        },
    }
}

/// Whether any deploy of this run claims the released destination.
pub fn covered(declared: &[Claim], dest: DestId, slot: &Slot) -> bool {
    let mut i = 0;
    while i < declared.len() {
        if covers(&declared[i], dest, slot) {
            return true;
        }
        i += 1;
    }
    false
}

/// What the ownership decision is asked to authorize: deploying the intent,
/// or a removal (a release is a removal, whatever it brings back).
pub fn asked(want: &Want) -> Intended {
    match want {
        Want::Deploy { ident, .. } => Intended::Obj(*ident),
        Want::Release(_) => Intended::Removed,
    }
}

/// What a write with this authority installs for the unit's want.
pub fn install(want: &Want, write: &Write) -> Option<Intended> {
    match want {
        Want::Deploy { ident, adopted } => match write {
            Write::TakeOver => Some(Intended::Obj(*adopted)),
            Write::Create => Some(Intended::Obj(*ident)),
            Write::Replace => Some(Intended::Obj(*ident)),
        },
        Want::Release(origin) => match write {
            Write::Replace => Some(*origin),
            Write::Create => None,
            Write::TakeOver => None,
        },
    }
}

fn written(want: &Want, write: Write) -> Step {
    match install(want, &write) {
        Some(intended) => Step::Write {
            install: intended,
            write,
        },
        // a release is authorized only as a removal of gripsack's own write
        None => Step::Refuse(Refusal::Malformed),
    }
}

fn stops(drift: &OnDrift) -> bool {
    match drift {
        OnDrift::Keep => false,
        OnDrift::Stop => true,
    }
}

/// Whether a refusal stops the run: a deploy cannot install over an object
/// without identity, and stops on the user's object when the unit says so;
/// a release always leaves the destination as it is.
fn refusal_stops(unit: &UnitFacts, why: &UnitRefusal) -> bool {
    match unit.want {
        Want::Release(_) => false,
        Want::Deploy { .. } => match why {
            UnitRefusal::Unsupported => true,
            UnitRefusal::Drifted => stops(&unit.drift),
            UnitRefusal::Unowned => stops(&unit.drift),
        },
    }
}

/// Decide one unit against the observed live object: the ownership decision
/// (K1) says whether the destination may change; the unit's want says what
/// it changes to.
pub fn decide_apply(index: u32, unit: &UnitFacts, live: &Live) -> Step {
    match decide_unit(&unit.recorded, live, &asked(&unit.want), &unit.consent) {
        UnitDecision::Satisfied => Step::Leave(Settled::Already),
        UnitDecision::Create => written(&unit.want, Write::Create),
        UnitDecision::ReplaceOwn => written(&unit.want, Write::Replace),
        UnitDecision::TakeOver => written(&unit.want, Write::TakeOver),
        UnitDecision::Refuse(why) => {
            if refusal_stops(unit, &why) {
                Step::Refuse(Refusal::Refused {
                    unit: index,
                    dest: unit.dest,
                    why,
                })
            } else {
                Step::Leave(Settled::Kept(why))
            }
        }
    }
}

fn refuse(mut state: ApplyState, refusal: Refusal) -> (ApplyState, Command) {
    state.phase = Phase::Aborted;
    (state, Command::Abort(refusal))
}

fn failed_or_protocol(state: ApplyState, event: Event) -> (ApplyState, Command) {
    match event {
        Event::Failed(_) => refuse(state, Refusal::Failed),
        _ => refuse(state, Refusal::Protocol),
    }
}

fn read_unit(mut state: ApplyState) -> (ApplyState, Command) {
    state.phase = Phase::ReadingUnit;
    let unit = state.index;
    (state, Command::ReadUnit(unit))
}

/// The unit under decision was reported: move to the next one.
fn advance(mut state: ApplyState) -> (ApplyState, Command) {
    state.unit = None;
    state.install = None;
    match state.index.checked_add(1) {
        None => refuse(state, Refusal::Malformed),
        Some(next) => {
            state.index = next;
            read_unit(state)
        }
    }
}

fn report(mut state: ApplyState, dest: DestId, settled: Settled) -> (ApplyState, Command) {
    state.phase = Phase::Reporting;
    let unit = state.index;
    (
        state,
        Command::Report {
            unit,
            dest,
            settled,
        },
    )
}

/// Examine the unit (observe it, or hand it to the host's domain).
fn examine(mut state: ApplyState, facts: UnitFacts) -> (ApplyState, Command) {
    let unit = state.index;
    let dest = facts.dest;
    let route = facts.route;
    state.unit = Some(facts);
    match route {
        Route::Delegated => {
            state.phase = Phase::Delegating;
            (state, Command::Delegate { unit, dest })
        }
        Route::Leaf => {
            state.phase = Phase::Observing;
            (state, Command::Observe { unit, dest })
        }
    }
}

fn on_unit(mut state: ApplyState, facts: UnitFacts) -> (ApplyState, Command) {
    match facts.want {
        Want::Deploy { .. } => {
            if state.releasing {
                let unit = state.index;
                return refuse(state, Refusal::Order { unit });
            }
            if state.declared.len() == usize::MAX {
                return refuse(state, Refusal::Malformed);
            }
            state.declared.push(Claim {
                dest: facts.dest,
                slot: facts.slot,
            });
            examine(state, facts)
        }
        Want::Release(_) => {
            state.releasing = true;
            if covered(&state.declared, facts.dest, &facts.slot) {
                let dest = facts.dest;
                state.unit = Some(facts);
                report(state, dest, Settled::Declared)
            } else {
                examine(state, facts)
            }
        }
    }
}

fn on_observed(mut state: ApplyState, dest: DestId, live: Live) -> (ApplyState, Command) {
    let facts = match state.unit {
        None => return refuse(state, Refusal::Protocol),
        Some(facts) => facts,
    };
    if !facts.dest.same(dest) {
        return refuse(state, Refusal::Protocol);
    }
    let unit = state.index;
    match decide_apply(unit, &facts, &live) {
        Step::Write { install, write } => {
            state.phase = Phase::Writing;
            state.install = Some(install);
            (
                state,
                Command::Mutate {
                    unit,
                    dest,
                    expect: live,
                    install,
                    write,
                },
            )
        }
        Step::Leave(settled) => report(state, dest, settled),
        Step::Refuse(refusal) => refuse(state, refusal),
    }
}

fn on_landed(mut state: ApplyState, live: Live) -> (ApplyState, Command) {
    match (state.unit, state.install) {
        (Some(facts), Some(install)) => {
            if lands(&install, &live) {
                state.mutated = true;
                report(state, facts.dest, Settled::Applied)
            } else {
                let unit = state.index;
                refuse(
                    state,
                    Refusal::NotApplied {
                        unit,
                        dest: facts.dest,
                    },
                )
            }
        }
        _ => refuse(state, Refusal::Protocol),
    }
}

fn on_write_failed(state: ApplyState, failure: Failure) -> (ApplyState, Command) {
    match failure {
        Failure::Io(_) => refuse(state, Refusal::Failed),
        Failure::Drift => match state.unit {
            None => refuse(state, Refusal::Protocol),
            Some(facts) => {
                let unit = state.index;
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

fn on_delegated(mut state: ApplyState, event: Event) -> (ApplyState, Command) {
    match state.unit {
        None => refuse(state, Refusal::Protocol),
        Some(facts) => match event {
            Event::Delegated { mutated } => {
                if mutated {
                    state.mutated = true;
                }
                report(state, facts.dest, Settled::Delegated { mutated })
            }
            Event::Refused => {
                let unit = state.index;
                refuse(
                    state,
                    Refusal::Delegated {
                        unit,
                        dest: facts.dest,
                    },
                )
            }
            _ => failed_or_protocol(state, event),
        },
    }
}

fn on_reading(mut state: ApplyState, event: Event) -> (ApplyState, Command) {
    match event {
        Event::UnitRead(facts) => on_unit(state, facts),
        Event::Exhausted => {
            state.phase = Phase::Directories;
            (state, Command::DirectoryStep)
        }
        _ => failed_or_protocol(state, event),
    }
}

/// The commit facts: the host's observation, and whether this run touched
/// the disk — which the machine knows itself.
pub fn commit_facts(inputs: &CommitInputs, mutated: bool) -> CommitFacts {
    CommitFacts {
        previous: inputs.previous,
        reserved: inputs.reserved,
        allocated: inputs.allocated,
        floor: inputs.floor,
        unchanged: inputs.unchanged,
        touched: mutated,
        exists: inputs.exists,
    }
}

fn on_commit(mut state: ApplyState, inputs: CommitInputs) -> (ApplyState, Command) {
    let facts = commit_facts(&inputs, state.mutated);
    match decide_commit(&facts) {
        Commit::Vacuous => {
            state.phase = Phase::Finished;
            (state, Command::Finish(Outcome::Vacuous))
        }
        Commit::Publish => {
            state.phase = Phase::Publishing;
            state.commit = Some(facts);
            (state, Command::Publish(facts.allocated))
        }
        Commit::Refuse(why) => refuse(state, Refusal::Unpublishable(why)),
    }
}

fn on_published(mut state: ApplyState) -> (ApplyState, Command) {
    match state.commit {
        None => refuse(state, Refusal::Protocol),
        Some(facts) => {
            state.phase = Phase::Flipping;
            (state, Command::Flip(facts.reserved))
        }
    }
}

/// One step: consume the answer to the previous command (or `Start`) and
/// return the next command. Any event that does not answer the outstanding
/// command aborts with [`Refusal::Protocol`]; a failed command aborts with
/// [`Refusal::Failed`] (or [`Refusal::Drifted`] for a write's re-check).
pub fn apply_step(state: ApplyState, event: Event) -> (ApplyState, Command) {
    match state.phase {
        Phase::Start => match event {
            Event::Start => read_unit(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingUnit => on_reading(state, event),
        Phase::Observing => match event {
            Event::Observed { dest, live } => on_observed(state, dest, live),
            _ => failed_or_protocol(state, event),
        },
        Phase::Writing => match event {
            Event::Landed { live } => on_landed(state, live),
            Event::Failed(failure) => on_write_failed(state, failure),
            _ => refuse(state, Refusal::Protocol),
        },
        Phase::Delegating => on_delegated(state, event),
        Phase::Reporting => match event {
            Event::Done => advance(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Directories => match event {
            Event::Done => {
                let mut next = state;
                next.phase = Phase::ReadingCommit;
                (next, Command::ReadCommit)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingCommit => match event {
            Event::CommitRead(inputs) => on_commit(state, inputs),
            _ => failed_or_protocol(state, event),
        },
        Phase::Publishing => match event {
            Event::Done => on_published(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Flipping => match event {
            Event::Done => {
                let mut next = state;
                next.phase = Phase::Activating;
                (next, Command::Activate)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::Activating => match event {
            Event::Done => {
                let mut next = state;
                next.phase = Phase::Finished;
                (next, Command::Finish(Outcome::Applied))
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::Finished | Phase::Aborted => refuse(state, Refusal::Protocol),
    }
}
