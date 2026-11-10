//! The GC machine (plan/0056 §4, F2 step 4): store collection as a sans-IO
//! state machine. The machine decides every deletion; the host executes
//! exactly the commands it is given.
//!
//! One run, in order:
//!
//! 1. **Inventory.** The host reports whether recovery or activation is
//!    pending, the `current` generation and the generation inventory.
//!    Collection is admitted only with nothing pending and `current` in the
//!    inventory ([`admit_gc`]); the prune plan is the oldest excess beyond
//!    `keep`, never `current` ([`plan_prune`]).
//! 2. **Manifests**, newest first, one per inventoried generation. A
//!    generation is retired only when the prune plan names it and no
//!    retained (newer) manifest names it as a predecessor; reading newest
//!    first makes that the closure in one pass. The whole decision is then
//!    checked closed ([`closure`]): every retained generation's
//!    predecessors are in the inventory and retained.
//! 3. **Roots** outside the generations (workspace and schedule roots,
//!    gathered by the host), then the **candidates**: every entry of
//!    `store/`, `prior/`, and every stale `generations/.staging-*`. The
//!    deletion set is the candidates no root names ([`plan_delete`]).
//! 4. **Plan**: the host reports (and sizes) the decision. A preview
//!    finishes here, having changed nothing.
//! 5. **Effects** (collect mode): seal the allocation floor; retire each
//!    retired generation (`N` → `.retiring-N`, newest first, after the host
//!    re-checks that `current` is still what the inventory said); one
//!    barrier on `generations/`; the directory domain's retirement of the
//!    receipts the retired generations held (delegated, plan/0056 F2
//!    step 9); then each deletion, after the host re-checks that the name
//!    still holds the inventoried object.
//!
//! Proved (proofs/Kernel/Gc.lean), relative to the `Inventory` event: no
//! effect follows a pending recovery or activation, or a preview; a retired
//! generation is in the prune plan — never `current` — and no retained
//! manifest names it; every inventoried generation's manifest was read, and
//! the retained ones are closed under predecessors; a removed candidate is
//! one no retained manifest pins and no external root names; and every
//! removal follows the generation barrier and the delegated retirement.
//!
//! Aeneas shape (lib.rs): no derives (impls in `convenience.rs`), equality as
//! explicit `same` functions, no closures; counters advance with
//! `checked_add`, so every step is total.
use crate::generation::{GenerationId, GenerationList};
use crate::recover::Failure;
use crate::retention::{
    Candidate, Fate, GcAdmission, Manifest, ManifestFacts, Stamp, Unclosed, admit_gc, closure,
    contains_generation, plan_delete, plan_prune, required,
};
use alloc::vec::Vec;

/// Whether the run may change anything.
pub enum Mode {
    /// Decide and report; never act (`grip gc --dry-run`).
    Preview,
    /// Decide, report, then act.
    Collect,
}

/// The state collection is decided against, as the host admitted it.
pub struct InventoryFacts {
    /// A run marker, journal entries, a quarantine or a pending activation
    /// exist.
    pub recovery_pending: bool,
    pub current: Option<GenerationId>,
    pub generations: GenerationList,
}

pub enum Outcome {
    /// The decision was reported; nothing changed.
    Previewed,
    /// Every decided effect was executed.
    Collected,
}

/// Why collection stopped. Effects issued before the refusal stand; every one
/// of them was decided by this run, and the next run decides afresh.
pub enum Refusal {
    /// Recovery or activation is pending: a journaled prior blob may be
    /// referenced by no manifest.
    RecoveryPending,
    /// `current` names a generation the inventory does not have.
    CorruptCurrent,
    /// A retained generation's predecessor is not in the inventory.
    MissingPredecessor { generation: GenerationId },
    /// The decided fates do not close (a retired generation is a retained
    /// one's predecessor), or a counter overflowed.
    Malformed,
    /// A re-check before an act found something other than what the
    /// command expected.
    Drifted,
    /// The host failed to execute a command.
    Failed,
    /// An event that does not answer the outstanding command.
    Protocol,
}

pub enum Command {
    ReadInventory,
    /// Admit this generation's manifest; answer `Manifest`.
    ReadManifest(GenerationId),
    /// Gather the roots outside the generations; answer `Roots`.
    ReadRoots,
    /// Inventory `store/`, `prior/` and stale staging; answer `Candidates`.
    ReadCandidates,
    /// The decision is final: report it; answer `Done`.
    Plan,
    /// Persist the allocation floor before any generation leaves the
    /// inventory; answer `Done`.
    SealFloor,
    /// Re-check that `current` is still `current`, then move the
    /// generation out of retained history (`N` → `.retiring-N`).
    Retire {
        generation: GenerationId,
        current: Option<GenerationId>,
    },
    /// Barrier on `generations/`: every retirement is durable.
    Barrier,
    /// The directory domain retires the receipts the retired generations
    /// held, and clears finished retirements.
    Delegate,
    /// Re-check that candidate `candidate` still holds `expect`, then
    /// remove its name (a directory recursively, anything else unlinked).
    Remove {
        candidate: usize,
        expect: Stamp,
    },
    Finish(Outcome),
    Abort(Refusal),
}

pub enum Event {
    Start,
    Inventory(InventoryFacts),
    Manifest(ManifestFacts),
    Roots(Vec<Vec<u8>>),
    Candidates(Vec<Candidate>),
    Done,
    Failed(Failure),
}

pub enum Phase {
    Start,
    ReadingInventory,
    ReadingManifests,
    ReadingRoots,
    ReadingCandidates,
    Planning,
    SealingFloor,
    Retiring,
    Sealing,
    Delegating,
    Removing,
    Finished,
    Aborted,
}

pub struct GcState {
    pub phase: Phase,
    pub mode: Mode,
    /// How many generations to keep (`None`: prune none).
    pub keep: Option<usize>,
    pub inventory: InventoryFacts,
    /// The prune plan.
    pub prune: Vec<GenerationId>,
    /// The manifests read so far, newest first, with their fates.
    pub manifests: Vec<Manifest>,
    pub external: Vec<Vec<u8>>,
    pub candidates: Vec<Candidate>,
    /// The positions of the candidates to delete, ascending.
    pub doomed: Vec<usize>,
    /// Reading manifests: how many generations remain unread. Retiring: the
    /// manifest under retirement. Removing: the deletion under way.
    pub index: usize,
}

impl GcState {
    pub fn new(mode: Mode, keep: Option<usize>) -> GcState {
        GcState {
            phase: Phase::Start,
            mode,
            keep,
            inventory: InventoryFacts {
                recovery_pending: true,
                current: None,
                generations: GenerationList::empty(),
            },
            prune: Vec::new(),
            manifests: Vec::new(),
            external: Vec::new(),
            candidates: Vec::new(),
            doomed: Vec::new(),
            index: 0,
        }
    }
}

/// The fate of a generation whose manifest is read after every newer one:
/// retired only when pruned and no retained newer manifest needs it.
pub fn fate(prune: &[GenerationId], manifests: &[Manifest], generation: GenerationId) -> Fate {
    if contains_generation(prune, generation) && !required(manifests, generation) {
        Fate::Retired
    } else {
        Fate::Retained
    }
}

fn refuse(mut state: GcState, refusal: Refusal) -> (GcState, Command) {
    state.phase = Phase::Aborted;
    (state, Command::Abort(refusal))
}

fn failed_or_protocol(state: GcState, event: Event) -> (GcState, Command) {
    match event {
        Event::Failed(failure) => match failure {
            Failure::Drift => refuse(state, Refusal::Drifted),
            Failure::Io(_) => refuse(state, Refusal::Failed),
        },
        _ => refuse(state, Refusal::Protocol),
    }
}

/// The next (older) manifest, or — every manifest read — the closure check
/// and the external roots.
fn next_manifest(mut state: GcState) -> (GcState, Command) {
    if state.index > 0 {
        let generation = state.inventory.generations.as_slice()[state.index - 1];
        state.phase = Phase::ReadingManifests;
        (state, Command::ReadManifest(generation))
    } else {
        match closure(&state.manifests, state.inventory.generations.as_slice()) {
            Some(Unclosed::MissingPredecessor { generation }) => {
                refuse(state, Refusal::MissingPredecessor { generation })
            }
            Some(Unclosed::Required { .. }) => refuse(state, Refusal::Malformed),
            None => {
                state.phase = Phase::ReadingRoots;
                (state, Command::ReadRoots)
            }
        }
    }
}

fn on_inventory(mut state: GcState, facts: InventoryFacts) -> (GcState, Command) {
    match admit_gc(facts.recovery_pending, facts.current, &facts.generations) {
        GcAdmission::RecoveryPending => refuse(state, Refusal::RecoveryPending),
        GcAdmission::CorruptCurrent => refuse(state, Refusal::CorruptCurrent),
        GcAdmission::Admitted => {
            state.prune = plan_prune(&facts.generations, facts.current, state.keep);
            state.index = facts.generations.as_slice().len();
            state.inventory = facts;
            next_manifest(state)
        }
    }
}

fn on_manifest(mut state: GcState, facts: ManifestFacts) -> (GcState, Command) {
    if state.index == 0 {
        return refuse(state, Refusal::Protocol);
    }
    let unread = state.index - 1;
    let expected = state.inventory.generations.as_slice()[unread];
    if !facts.id.same(expected) {
        return refuse(state, Refusal::Protocol);
    }
    let decided = fate(&state.prune, &state.manifests, facts.id);
    state.manifests.push(Manifest {
        facts,
        fate: decided,
    });
    state.index = unread;
    next_manifest(state)
}

fn on_candidates(mut state: GcState, candidates: Vec<Candidate>) -> (GcState, Command) {
    state.doomed = plan_delete(&state.manifests, &state.external, &candidates);
    state.candidates = candidates;
    state.phase = Phase::Planning;
    (state, Command::Plan)
}

/// The first retired manifest at or after `from`.
pub fn next_retired(manifests: &[Manifest], from: usize) -> Option<usize> {
    let mut i = from;
    while i < manifests.len() {
        if !manifests[i].fate.retained() {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Retire the next retired generation (newest first), or seal the
/// generations directory once none is left.
fn retire_next(mut state: GcState) -> (GcState, Command) {
    match next_retired(&state.manifests, state.index) {
        Some(position) => {
            let generation = state.manifests[position].facts.id;
            let current = state.inventory.current;
            state.index = position;
            state.phase = Phase::Retiring;
            (
                state,
                Command::Retire {
                    generation,
                    current,
                },
            )
        }
        None => {
            state.phase = Phase::Sealing;
            (state, Command::Barrier)
        }
    }
}

/// Remove the next doomed candidate, or finish.
fn remove_next(mut state: GcState) -> (GcState, Command) {
    if state.index < state.doomed.len() {
        let candidate = state.doomed[state.index];
        if candidate < state.candidates.len() {
            let expect = state.candidates[candidate].stamp;
            state.phase = Phase::Removing;
            (state, Command::Remove { candidate, expect })
        } else {
            refuse(state, Refusal::Protocol)
        }
    } else {
        state.phase = Phase::Finished;
        (state, Command::Finish(Outcome::Collected))
    }
}

/// One step: consume the answer to the previous command (or `Start`) and
/// return the next command. Any event that does not answer the outstanding
/// command aborts with [`Refusal::Protocol`]; a failed command aborts with
/// [`Refusal::Failed`] (or [`Refusal::Drifted`] for a re-check).
pub fn gc_step(state: GcState, event: Event) -> (GcState, Command) {
    match state.phase {
        Phase::Start => match event {
            Event::Start => {
                let mut next = state;
                next.phase = Phase::ReadingInventory;
                (next, Command::ReadInventory)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingInventory => match event {
            Event::Inventory(facts) => on_inventory(state, facts),
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingManifests => match event {
            Event::Manifest(facts) => on_manifest(state, facts),
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingRoots => match event {
            Event::Roots(external) => {
                let mut next = state;
                next.external = external;
                next.phase = Phase::ReadingCandidates;
                (next, Command::ReadCandidates)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingCandidates => match event {
            Event::Candidates(candidates) => on_candidates(state, candidates),
            _ => failed_or_protocol(state, event),
        },
        Phase::Planning => match event {
            Event::Done => {
                let mut next = state;
                match next.mode {
                    Mode::Preview => {
                        next.phase = Phase::Finished;
                        (next, Command::Finish(Outcome::Previewed))
                    }
                    Mode::Collect => {
                        next.phase = Phase::SealingFloor;
                        (next, Command::SealFloor)
                    }
                }
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::SealingFloor => match event {
            Event::Done => {
                let mut next = state;
                next.index = 0;
                retire_next(next)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::Retiring => match event {
            Event::Done => match state.index.checked_add(1) {
                None => refuse(state, Refusal::Malformed),
                Some(index) => {
                    let mut next = state;
                    next.index = index;
                    retire_next(next)
                }
            },
            _ => failed_or_protocol(state, event),
        },
        Phase::Sealing => match event {
            Event::Done => {
                let mut next = state;
                next.phase = Phase::Delegating;
                (next, Command::Delegate)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::Delegating => match event {
            Event::Done => {
                let mut next = state;
                next.index = 0;
                remove_next(next)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::Removing => match event {
            Event::Done => match state.index.checked_add(1) {
                None => refuse(state, Refusal::Malformed),
                Some(index) => {
                    let mut next = state;
                    next.index = index;
                    remove_next(next)
                }
            },
            _ => failed_or_protocol(state, event),
        },
        Phase::Finished | Phase::Aborted => refuse(state, Refusal::Protocol),
    }
}
