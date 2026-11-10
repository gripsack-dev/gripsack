//! Generation numbering and publication (plan/0056 K6, F2 step 5).
//!
//! Generations are immutable history: a number names one published state
//! forever, so a new generation must take a number no generation ever had,
//! even after gc retired the newest ones or a rollback selected an older one.
//! The durable high-water floor (`generations/high-water`) records the
//! highest number ever allocated; the host reads it with `current` and the
//! newest generation on disk and asks [`allocate`]. Before a generation is
//! renamed into place (and before gc retires the newest), the floor is moved
//! up, never down ([`floor_action`]). At the end of an apply,
//! [`decide_commit`] decides whether the run cuts a generation at all — a
//! vacuous run (nothing changed, nothing touched) cuts none — and admits the
//! publication and its flip only for the allocated number, above the floor,
//! not already published, under a reserved selection whose transaction is
//! fresh.
//!
//! Proved (proofs/Kernel/Publication.lean): an allocated number exceeds the
//! current generation, the newest on disk and the floor; the floor action
//! never lowers the floor; a publication is admitted only for a fresh
//! transaction bound to the allocated generation, above the floor and not
//! already published; a vacuous run publishes nothing, and a run that changed
//! the state or touched the disk is never vacuous.
use crate::GenerationId;
use crate::selection::{SelectionIdentity, fresh};

/// What the allocator observed.
pub struct Allocation {
    /// The generation `current` selects.
    pub current: Option<GenerationId>,
    /// The newest generation directory on disk.
    pub newest: Option<GenerationId>,
    /// The durable high-water floor.
    pub floor: Option<GenerationId>,
}

/// The larger of two optional generations.
pub fn later(a: Option<GenerationId>, b: Option<GenerationId>) -> Option<GenerationId> {
    match (a, b) {
        (None, None) => None,
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (Some(a), Some(b)) => {
            if a.value() < b.value() {
                Some(b)
            } else {
                Some(a)
            }
        }
    }
}

/// `number` is strictly above `floor` (anything is above no floor). (Not
/// called `generation`: a local named like a module shadows it in the
/// translation.)
pub fn above(number: GenerationId, floor: Option<GenerationId>) -> bool {
    match floor {
        None => true,
        Some(floor) => floor.value() < number.value(),
    }
}

/// The next generation number: one above everything observed (`1` on a
/// fresh home), or `None` once the numbers are exhausted.
pub fn allocate(observed: &Allocation) -> Option<GenerationId> {
    match later(later(observed.current, observed.newest), observed.floor) {
        None => Some(GenerationId::new(1)),
        Some(top) => top.checked_next(),
    }
}

/// What to do with the floor before publishing (or retiring) names.
pub enum FloorAction {
    /// No generation was ever allocated: nothing to keep.
    Nothing,
    /// The observed floor already covers everything retained: make it
    /// durable as it is.
    Seal,
    /// Write this floor durably.
    Raise(GenerationId),
}

/// Keep every admitted allocation: the floor becomes the larger of what it
/// says and the highest retained (or about to be published) generation. It
/// never moves down.
pub fn floor_action(observed: Option<GenerationId>, retained: Option<GenerationId>) -> FloorAction {
    match later(observed, retained) {
        None => FloorAction::Nothing,
        Some(floor) => match observed {
            Some(observed) => {
                if observed.same(floor) {
                    FloorAction::Seal
                } else {
                    FloorAction::Raise(floor)
                }
            }
            None => FloorAction::Raise(floor),
        },
    }
}

/// What the end of an apply observed.
pub struct CommitFacts {
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
    /// The run changed something on disk.
    pub touched: bool,
    /// A generation directory with the allocated number already exists.
    pub exists: bool,
}

pub enum CommitRefusal {
    /// The reserved selection does not carry a fresh transaction.
    Reused,
    /// The reserved selection names another generation.
    Misbound,
    /// The allocated number does not exceed the floor.
    BelowFloor,
    /// The allocated number is already published.
    Exists,
}

pub enum Commit {
    /// Nothing changed: no generation, no flip.
    Vacuous,
    /// Publish generation `allocated`, then flip to `reserved`.
    Publish,
    Refuse(CommitRefusal),
}

/// Whether and how an apply commits.
pub fn decide_commit(facts: &CommitFacts) -> Commit {
    if vacuous(facts) {
        Commit::Vacuous
    } else {
        admit_publication(facts)
    }
}

/// A previous generation exists, the state is unchanged and the run touched
/// nothing on disk.
pub fn vacuous(facts: &CommitFacts) -> bool {
    match facts.previous {
        Some(_) => facts.unchanged && !facts.touched,
        None => false,
    }
}

/// Publication is admitted only for the allocated number, above the floor,
/// not yet published, under a fresh reserved transaction bound to it.
pub fn admit_publication(facts: &CommitFacts) -> Commit {
    if !fresh(&facts.reserved, &facts.previous) {
        Commit::Refuse(CommitRefusal::Reused)
    } else if !facts.reserved.generation().same(facts.allocated) {
        Commit::Refuse(CommitRefusal::Misbound)
    } else if !above(facts.allocated, facts.floor) {
        Commit::Refuse(CommitRefusal::BelowFloor)
    } else if facts.exists {
        Commit::Refuse(CommitRefusal::Exists)
    } else {
        Commit::Publish
    }
}
