//! The rollback machine (plan/0056 §4, F2 step 8): reselecting a retained
//! generation as a sans-IO state machine.
//!
//! Rollback walks every ownership unit the current and the target
//! generation deploy (the host orders them, deepest destination first) and
//! settles each one before it reselects the target. Per unit, from the
//! target generation's record, gripsack's record in the current generation
//! and one observation of the live object, it decides ([`decide_restore`]):
//!
//! - **install** the target's recorded identity (bytes plus mode, or a link
//!   target) when the destination is absent or holds gripsack's own current
//!   write, and only from retained bytes the host verified reproduce that
//!   identity; or **release** gripsack's own write where the target deploys
//!   nothing (remove it, or bring back the object the ownership epoch
//!   recorded before gripsack took the destination over);
//! - **leave** a destination that already holds the target's identity, that
//!   the user changed (kept, per K1: the ownership decision [`decide_unit`]
//!   without consent), or that the target itself recorded as preserved drift;
//! - **refuse** the whole rollback when a write is needed and no retained
//!   bytes reproduce the record (an older receipt, a template whose recorded
//!   variables no longer render it), or when the write would land inside a
//!   published generation's directory.
//!
//! Units the host settles itself (managed blocks) are delegated and reported
//! settled or unrestorable. The directory domain is a typed sub-protocol
//! (step 9, [`crate::directory`]): its receipts are bound, the gripsack
//! files a shape change needs gone are cleared as `Cleared` units decided
//! here (gripsack's own write is removed, anything else refuses), shapes are
//! changed by commanded steps, the target's nodes are held (a unit there is
//! the domain's, and one the target writes there aborts), and a receipt the
//! target does not keep is retired once every unit settled. Items come in
//! stage order; an item out of order aborts.
//!
//! The selection flips only after every unit was reported settled, only to
//! the target under a transaction that differs from the one being replaced,
//! and only when the target retains its own shell profile: rollback never
//! writes into an existing generation (FND-2).
//!
//! Proved (proofs/Kernel/Rollback.lean): every write installs exactly the
//! target generation's recorded identity for the unit under decision (K2),
//! into absence or over gripsack's own current write (K1, no consent), never
//! inside a generation directory, and a unit is reported restored only when
//! the landed object is that identity; no write targets a destination the
//! directory domain holds, and no directory step is commanded for a
//! destination whose target record a unit writes; a clear only removes
//! gripsack's own write; the flip is issued only after every unit and every
//! directory step was reported settled, and carries the planned selection
//! with a fresh transaction. Relative to premise D of [`crate::directory`].
//!
//! Aeneas shape (lib.rs): no derives (impls in `convenience.rs`), equality as
//! explicit `same` functions, owned `Copy` facts, no closures; counters
//! advance with `checked_add`, so every step is total.
use crate::directory::{Action, DirectoryFacts, Stage, commands, names, ordered, stage};
use crate::ids::DestId;
use crate::ownership::{
    Consent, Decision as UnitDecision, Recorded, Refusal as UnitRefusal, TakeOver, decide_unit,
};
use crate::recover::{Failure, Ident, Intended, Live, Route};
use crate::selection::SelectionIdentity;
use alloc::vec::Vec;

/// Where a unit's destination lies, as the host classified its canonical
/// path.
pub enum Place {
    /// An ordinary destination, outside gripsack's own state.
    Outside,
    /// Inside `$GRIPSACK_HOME/generations`: published generations are
    /// immutable, so rollback never writes there.
    Generation,
}

/// Whether the host holds retained bytes that reproduce a recorded
/// identity exactly (store bytes, re-rendered older templates and the
/// recorded mode, checked against the record before the machine sees it).
pub enum Bytes {
    Retained,
    Missing,
}

/// What the target generation records at this unit.
pub enum Target {
    /// The target deploys nothing here: gripsack releases the destination.
    /// Its own write goes, replaced by what the ownership epoch recorded
    /// before gripsack took the destination over (`Obj`), or by nothing
    /// (`Removed`).
    Released(Intended),
    /// The target recorded writing this identity.
    Wrote { ident: Ident, bytes: Bytes },
    /// The target kept a user's object here (preserved drift): it records
    /// no write, and rollback leaves the destination alone.
    Preserved,
    /// A directory shape change needs this destination empty: gripsack's own
    /// current write goes and nothing comes back. The user's object, or an
    /// ownership epoch that would restore an origin (`Obj`), refuses the
    /// rollback. (A later unit may install the target's record here again,
    /// in the new shape.)
    Cleared(Intended),
}

/// When a unit is settled relative to the directory shape changes.
pub enum When {
    /// Before: its destination lies where a target receipt's shape change
    /// takes the current namespace away (a clear, or a release below a
    /// borrowed directory the target no longer keeps).
    BeforeShape,
    /// After the shape changes and the holds (every other unit).
    AfterShape,
}

/// One unit, as the host decoded it from both manifests.
pub struct UnitFacts {
    pub dest: DestId,
    pub place: Place,
    pub route: Route,
    /// gripsack's record in the current generation (its lineage).
    pub current: Recorded,
    pub target: Target,
    pub when: When,
}

/// Whether the target generation retains the shell profile its selection
/// activates (an older generation may predate in-generation profiles).
pub enum Profile {
    Retained,
    Missing,
}

/// The rollback as the host prepared it.
pub struct PlanFacts {
    pub units: u32,
    /// The target generation under the transaction this run reserved.
    pub reselect: SelectionIdentity,
    /// The selection `current` names now.
    pub previous: Option<SelectionIdentity>,
    pub profile: Profile,
}

/// How a unit was settled (the host reports it).
pub enum Settled {
    /// The target's recorded identity landed.
    Restored,
    /// The destination already held the target's identity (or its removal).
    Already,
    /// The user's object stands; nothing was written (K1).
    Kept(UnitRefusal),
    /// The target recorded preserved drift: the destination is not
    /// gripsack's to restore.
    Untouched,
    /// The host's own domain (managed blocks) settled it.
    Delegated,
    /// The directory domain holds the destination (a node of a target
    /// receipt): nothing for the leaf protocol to restore; a `Hold` item
    /// records the hold.
    Held,
    /// A directory step settled.
    Directory,
}

/// The per-unit decision.
pub enum Restore {
    /// Install this (the target's recorded identity, or the removal).
    Write(Intended),
    /// Nothing to write.
    Leave(Settled),
    /// Stop the rollback before the flip.
    Refuse(Refusal),
}

pub enum Outcome {
    /// Every unit settled and the target was reselected.
    RolledBack,
}

/// Why rollback stopped. Nothing is reselected; the host compensates any
/// write this run made from the journal.
pub enum Refusal {
    /// The target generation has no shell profile of its own; restoring it
    /// would mean writing into an existing generation.
    ProfileMissing,
    /// The reserved selection is not a fresh transaction.
    Reused,
    /// A write is needed and no retained bytes reproduce the target's
    /// record (or the host's domain cannot restore a delegated unit).
    Unrestorable { unit: u32, dest: DestId },
    /// The write would land inside a published generation's directory.
    Protected { unit: u32, dest: DestId },
    /// The live object changed between the decision and the act.
    Drifted { unit: u32, dest: DestId },
    /// The write did not land the intended identity.
    NotRestored { unit: u32, dest: DestId },
    /// An item out of stage order.
    Order { unit: u32 },
    /// A unit that would write or clear a destination the directory domain
    /// holds, or a directory step at a destination whose target record a
    /// unit writes.
    Overlap { unit: u32, dest: DestId },
    /// A clear that may not empty its destination (the user's object, or an
    /// origin its epoch would restore).
    Blocked { unit: u32, dest: DestId },
    /// The directory sub-protocol could not settle a step.
    Directory { unit: u32, dest: DestId },
    /// A counter overflowed.
    Malformed,
    /// The host failed to execute a command.
    Failed,
    /// An event that does not answer the outstanding command.
    Protocol,
}

pub enum Command {
    ReadPlan,
    ReadUnit(u32),
    Observe {
        unit: u32,
        dest: DestId,
    },
    /// Re-check `expect` against the live object immediately before acting
    /// (answer `Failed(Drift)` on mismatch), install `install`, and answer
    /// `Landed` with the observed result.
    Mutate {
        unit: u32,
        dest: DestId,
        expect: Live,
        install: Intended,
    },
    /// Settle a unit in the host's own domain: answer `Done` when settled,
    /// `Refused` when it cannot be restored.
    Delegate {
        unit: u32,
        dest: DestId,
    },
    /// Run one step of the directory sub-protocol for the receipt at
    /// `dest`: answer `Done` when settled, `Refused` when it cannot be.
    Directory {
        unit: u32,
        dest: DestId,
        action: Action,
    },
    /// Record how a unit was settled (the host's note); answer `Done`.
    Report {
        unit: u32,
        dest: DestId,
        settled: Settled,
    },
    /// Reselect: point `current` at this selection.
    Flip(SelectionIdentity),
    Finish(Outcome),
    Abort(Refusal),
}

pub enum Event {
    Start,
    PlanRead(PlanFacts),
    UnitRead(UnitFacts),
    /// The next item is a directory step (or hold).
    DirectoryRead(DirectoryFacts),
    Observed {
        dest: DestId,
        live: Live,
    },
    Landed {
        live: Live,
    },
    Done,
    /// A delegated unit cannot be restored, or a directory step cannot be
    /// settled.
    Refused,
    Failed(Failure),
}

pub enum Phase {
    Start,
    ReadingPlan,
    ReadingUnit,
    Observing,
    Writing,
    Delegating,
    Directing,
    Reporting,
    Flipping,
    Finished,
    Aborted,
}

pub struct RollbackState {
    pub phase: Phase,
    pub plan: PlanFacts,
    /// The unit under decision.
    pub index: u32,
    pub unit: Option<UnitFacts>,
    /// What the outstanding write installs.
    pub install: Option<Intended>,
    /// The stage of the last item read.
    pub stage: Stage,
    /// Every destination the directory domain holds.
    pub holds: Vec<DestId>,
    /// Every destination a unit wrote the target's record to (or will).
    pub claimed: Vec<DestId>,
    /// The directory item under execution.
    pub step: Option<DirectoryFacts>,
}

impl RollbackState {
    pub fn new() -> RollbackState {
        RollbackState {
            phase: Phase::Start,
            plan: PlanFacts {
                units: 0,
                reselect: SelectionIdentity::Legacy {
                    generation: crate::GenerationId::new(0),
                },
                previous: None,
                profile: Profile::Missing,
            },
            index: 0,
            unit: None,
            install: None,
            stage: Stage::Opening,
            holds: Vec::new(),
            claimed: Vec::new(),
            step: None,
        }
    }
}

/// Rollback never absorbs: consent is withheld for every unit.
fn withheld() -> Consent {
    Consent {
        take_over: TakeOver::Withheld,
    }
}

/// What the ownership decision is asked to authorize: deploying the
/// target's identity, or a removal (a release is a removal, exactly as
/// apply's prune decides it, whatever it brings back).
fn authority(target: &Target) -> Option<Intended> {
    match target {
        Target::Released(_) => Some(Intended::Removed),
        Target::Wrote { ident, .. } => Some(Intended::Obj(*ident)),
        Target::Preserved => None,
        Target::Cleared(_) => Some(Intended::Removed),
    }
}

/// What a write for this target installs.
pub fn install(target: &Target) -> Option<Intended> {
    match target {
        Target::Released(origin) => Some(*origin),
        Target::Wrote { ident, .. } => Some(Intended::Obj(*ident)),
        Target::Preserved => None,
        Target::Cleared(origin) => match origin {
            Intended::Removed => Some(Intended::Removed),
            Intended::Obj(_) => None,
        },
    }
}

/// Whether the host can install the target's identity exactly.
fn retained(target: &Target) -> bool {
    match target {
        Target::Released(_) => true,
        Target::Wrote { bytes, .. } => match bytes {
            Bytes::Retained => true,
            Bytes::Missing => false,
        },
        Target::Preserved => false,
        Target::Cleared(_) => true,
    }
}

/// Whether the unit clears its destination for a directory change.
fn clears(target: &Target) -> bool {
    match target {
        Target::Cleared(_) => true,
        Target::Released(_) => false,
        Target::Wrote { .. } => false,
        Target::Preserved => false,
    }
}

fn blocked(index: u32, unit: &UnitFacts) -> Restore {
    Restore::Refuse(Refusal::Blocked {
        unit: index,
        dest: unit.dest,
    })
}

fn writable(place: &Place) -> bool {
    match place {
        Place::Outside => true,
        Place::Generation => false,
    }
}

/// Decide one unit against the observed live object. The ownership decision
/// (K1, no consent) says whether the destination may change; the target's
/// record says what it changes to; the host's verification says whether that
/// can be done exactly.
pub fn decide_restore(index: u32, unit: &UnitFacts, live: &Live) -> Restore {
    match authority(&unit.target) {
        None => Restore::Leave(Settled::Untouched),
        Some(asked) => match decide_unit(&unit.current, live, &asked, &withheld()) {
            UnitDecision::Satisfied => Restore::Leave(Settled::Already),
            UnitDecision::Refuse(why) => {
                if clears(&unit.target) {
                    blocked(index, unit)
                } else {
                    Restore::Leave(Settled::Kept(why))
                }
            }
            UnitDecision::Create | UnitDecision::ReplaceOwn | UnitDecision::TakeOver => {
                writing(index, unit)
            }
        },
    }
}

/// A write the ownership decision authorized: exactly the target's
/// identity, from retained bytes, outside generation directories.
fn writing(index: u32, unit: &UnitFacts) -> Restore {
    match install(&unit.target) {
        None => {
            if clears(&unit.target) {
                blocked(index, unit)
            } else {
                Restore::Leave(Settled::Untouched)
            }
        }
        Some(intended) => {
            if !retained(&unit.target) {
                Restore::Refuse(Refusal::Unrestorable {
                    unit: index,
                    dest: unit.dest,
                })
            } else if !writable(&unit.place) {
                Restore::Refuse(Refusal::Protected {
                    unit: index,
                    dest: unit.dest,
                })
            } else {
                Restore::Write(intended)
            }
        }
    }
}

/// Whether `live` is what `install` intended.
pub fn lands(install: &Intended, live: &Live) -> bool {
    match (install, live) {
        (Intended::Removed, Live::Absent) => true,
        (Intended::Obj(ident), Live::Obj(object)) => ident.same(object),
        _ => false,
    }
}

/// The plan may be executed: the target keeps its own profile, and the
/// reserved selection is a transaction that differs from the current one.
pub fn admit(plan: &PlanFacts) -> Option<Refusal> {
    match plan.profile {
        Profile::Missing => Some(Refusal::ProfileMissing),
        Profile::Retained => {
            if crate::selection::fresh(&plan.reselect, &plan.previous) {
                None
            } else {
                Some(Refusal::Reused)
            }
        }
    }
}

fn refuse(mut state: RollbackState, refusal: Refusal) -> (RollbackState, Command) {
    state.phase = Phase::Aborted;
    (state, Command::Abort(refusal))
}

fn failed_or_protocol(state: RollbackState, event: Event) -> (RollbackState, Command) {
    match event {
        Event::Failed(_) => refuse(state, Refusal::Failed),
        _ => refuse(state, Refusal::Protocol),
    }
}

/// The next unit, or the flip once every unit is settled.
fn proceed(mut state: RollbackState) -> (RollbackState, Command) {
    if state.index < state.plan.units {
        state.phase = Phase::ReadingUnit;
        let unit = state.index;
        (state, Command::ReadUnit(unit))
    } else {
        state.phase = Phase::Flipping;
        let reselect = state.plan.reselect;
        (state, Command::Flip(reselect))
    }
}

/// The unit under decision was reported: move to the next one.
fn advance(mut state: RollbackState) -> (RollbackState, Command) {
    state.unit = None;
    state.install = None;
    state.step = None;
    match state.index.checked_add(1) {
        None => refuse(state, Refusal::Malformed),
        Some(next) => {
            state.index = next;
            proceed(state)
        }
    }
}

fn report(mut state: RollbackState, dest: DestId, settled: Settled) -> (RollbackState, Command) {
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

fn on_plan(mut state: RollbackState, plan: PlanFacts) -> (RollbackState, Command) {
    match admit(&plan) {
        Some(refusal) => refuse(state, refusal),
        None => {
            state.plan = plan;
            state.index = 0;
            proceed(state)
        }
    }
}

/// The stage a unit belongs to: a clear, or any unit the host settles
/// before the shape changes, comes in the clearing stage.
pub fn unit_stage(target: &Target, when: &When) -> Stage {
    if clears(target) {
        Stage::Clearing
    } else {
        match when {
            When::BeforeShape => Stage::Clearing,
            When::AfterShape => Stage::Units,
        }
    }
}

/// Whether the target records a write here: the unit claims the
/// destination for the leaf protocol.
fn records_write(target: &Target) -> bool {
    match target {
        Target::Wrote { .. } => true,
        Target::Cleared(_) => false,
        Target::Released(_) => false,
        Target::Preserved => false,
    }
}

/// Whether the unit would write the target's record or clear the
/// destination (a write the directory domain's holds forbid).
fn writes_target(target: &Target) -> bool {
    match target {
        Target::Wrote { .. } => true,
        Target::Cleared(_) => true,
        Target::Released(_) => false,
        Target::Preserved => false,
    }
}

fn on_unit(mut state: RollbackState, facts: UnitFacts) -> (RollbackState, Command) {
    let unit = state.index;
    let dest = facts.dest;
    let next = unit_stage(&facts.target, &facts.when);
    if !ordered(&state.stage, &next) {
        return refuse(state, Refusal::Order { unit });
    }
    state.stage = next;
    if names(&state.holds, dest) {
        if writes_target(&facts.target) {
            return refuse(state, Refusal::Overlap { unit, dest });
        }
        state.unit = Some(facts);
        return report(state, dest, Settled::Held);
    }
    if records_write(&facts.target) {
        if state.claimed.len() == usize::MAX {
            return refuse(state, Refusal::Malformed);
        }
        state.claimed.push(dest);
    }
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

/// A directory item: command its step (or record its hold), in stage order,
/// never for a destination whose target record a unit writes.
fn on_directory(mut state: RollbackState, facts: DirectoryFacts) -> (RollbackState, Command) {
    let unit = state.index;
    let dest = facts.dest;
    let next = stage(&facts.action);
    if !ordered(&state.stage, &next) {
        return refuse(state, Refusal::Order { unit });
    }
    state.stage = next;
    if names(&state.claimed, dest) {
        return refuse(state, Refusal::Overlap { unit, dest });
    }
    let action = facts.action;
    state.step = Some(facts);
    if commands(&action) {
        state.phase = Phase::Directing;
        (state, Command::Directory { unit, dest, action })
    } else {
        if state.holds.len() == usize::MAX {
            return refuse(state, Refusal::Malformed);
        }
        state.holds.push(dest);
        report(state, dest, Settled::Held)
    }
}

fn on_directed(state: RollbackState, event: Event) -> (RollbackState, Command) {
    match state.step {
        None => refuse(state, Refusal::Protocol),
        Some(facts) => match event {
            Event::Done => report(state, facts.dest, Settled::Directory),
            Event::Refused => {
                let unit = state.index;
                refuse(
                    state,
                    Refusal::Directory {
                        unit,
                        dest: facts.dest,
                    },
                )
            }
            _ => failed_or_protocol(state, event),
        },
    }
}

fn on_observed(mut state: RollbackState, dest: DestId, live: Live) -> (RollbackState, Command) {
    let facts = match state.unit {
        None => return refuse(state, Refusal::Protocol),
        Some(facts) => facts,
    };
    if !facts.dest.same(dest) {
        return refuse(state, Refusal::Protocol);
    }
    let unit = state.index;
    match decide_restore(unit, &facts, &live) {
        Restore::Write(install) => {
            state.phase = Phase::Writing;
            state.install = Some(install);
            (
                state,
                Command::Mutate {
                    unit,
                    dest,
                    expect: live,
                    install,
                },
            )
        }
        Restore::Leave(settled) => report(state, dest, settled),
        Restore::Refuse(refusal) => refuse(state, refusal),
    }
}

fn on_landed(state: RollbackState, live: Live) -> (RollbackState, Command) {
    match (state.unit, state.install) {
        (Some(facts), Some(install)) => {
            if lands(&install, &live) {
                report(state, facts.dest, Settled::Restored)
            } else {
                let unit = state.index;
                refuse(
                    state,
                    Refusal::NotRestored {
                        unit,
                        dest: facts.dest,
                    },
                )
            }
        }
        _ => refuse(state, Refusal::Protocol),
    }
}

fn on_write_failed(state: RollbackState, failure: Failure) -> (RollbackState, Command) {
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

fn on_delegated(state: RollbackState, event: Event) -> (RollbackState, Command) {
    match state.unit {
        None => refuse(state, Refusal::Protocol),
        Some(facts) => match event {
            Event::Done => report(state, facts.dest, Settled::Delegated),
            Event::Refused => {
                let unit = state.index;
                refuse(
                    state,
                    Refusal::Unrestorable {
                        unit,
                        dest: facts.dest,
                    },
                )
            }
            _ => failed_or_protocol(state, event),
        },
    }
}

/// One step: consume the answer to the previous command (or `Start`) and
/// return the next command. Any event that does not answer the outstanding
/// command aborts with [`Refusal::Protocol`]; a failed command aborts with
/// [`Refusal::Failed`] (or [`Refusal::Drifted`] for a write's re-check).
pub fn rollback_step(state: RollbackState, event: Event) -> (RollbackState, Command) {
    match state.phase {
        Phase::Start => match event {
            Event::Start => {
                let mut next = state;
                next.phase = Phase::ReadingPlan;
                (next, Command::ReadPlan)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingPlan => match event {
            Event::PlanRead(plan) => on_plan(state, plan),
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingUnit => match event {
            Event::UnitRead(facts) => on_unit(state, facts),
            Event::DirectoryRead(facts) => on_directory(state, facts),
            _ => failed_or_protocol(state, event),
        },
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
        Phase::Directing => on_directed(state, event),
        Phase::Reporting => match event {
            Event::Done => advance(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Flipping => match event {
            Event::Done => {
                let mut next = state;
                next.phase = Phase::Finished;
                (next, Command::Finish(Outcome::RolledBack))
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::Finished | Phase::Aborted => refuse(state, Refusal::Protocol),
    }
}
