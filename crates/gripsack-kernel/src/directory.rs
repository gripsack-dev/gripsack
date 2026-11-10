//! The directory sub-protocol (plan/0056 F2 step 9): the boundary between
//! the kernel's apply and rollback machines and the host's directory domain.
//!
//! The directory domain (journal v3 receipts: a migrated owned link that
//! became a real directory tree, its evolutions, retirement and
//! reactivation) stays in the host — moving it into the kernel is F5 work.
//! What the kernel owns is *when* it runs and *what it may touch*:
//!
//! - every directory mutation is a [`Action`] the machine commands for one
//!   receipt, named by its root destination, and the host answers whether it
//!   settled (and changed the disk) or refused. Nothing else in a run touches
//!   a directory receipt's namespace;
//! - the leaf removals a directory change needs first (a gripsack file where
//!   a new directory node must go, the children of a node being replaced)
//!   are ordinary whole-object units of the machine (`Clear`), decided by the
//!   proved ownership decision: gripsack's own write is removed, anything
//!   else refuses the run;
//! - the destinations the domain holds for the rest of the run (every owned
//!   node of every active receipt) are declared to the machine
//!   ([`Action::Hold`]): no whole-object unit ever writes one, and a release
//!   at one is the domain's, never the leaf protocol's;
//! - items arrive in [`Stage`] order — receipts opened, blockers cleared,
//!   shapes changed, nodes held, units settled, receipts closed — and an item
//!   out of order aborts the run.
//!
//! **Premise D (the sub-protocol's contract, not proved here).** A commanded
//! step changes only the namespace of the receipt its destination names: its
//! owned nodes, their private staging siblings, and the predecessor link at
//! its root (which `Retire` restores and then releases, or reconciles with
//! the target's record in a rollback; `Reactivate` replaces gripsack's own
//! object there or the receipt's recorded origin, never a foreign one). It
//! journals its record durably before its first change, and an interrupted
//! step is recovered only through the recover machine's delegated hooks
//! (`recover::Delegated`). A step answered settled left that namespace in
//! the receipt's shape; one answered refused may have left a journaled
//! partial change, which the run's compensation (journal reconcile) undoes.
//! The kernel theorems about apply and rollback hold relative to D, as the
//! ring-1 theorems hold relative to S1–S7.
//!
//! Aeneas shape (lib.rs): no derives, explicit loops, owned `Copy` facts.
use crate::ids::DestId;

/// Where an item belongs in a run. Items are handed to a machine in stage
/// order; within a stage the host orders them.
pub enum Stage {
    /// Receipts carried into the run (bound), born (migration) or planned.
    Opening,
    /// Whole-object units removing what blocks a directory change.
    Clearing,
    /// Directory shapes changed (evolution, rollback's shape, exchange,
    /// reactivation).
    Shaping,
    /// The nodes the domain holds from here on.
    Holding,
    /// Whole-object units (apply's deploys; every rollback unit).
    Units,
    /// Apply's releases (prune-on-undeclare).
    Releases,
    /// Receipts finished or retired, once every leaf settled.
    Closing,
}

/// A stage's position in a run.
pub fn rank(stage: &Stage) -> u8 {
    match stage {
        Stage::Opening => 0,
        Stage::Clearing => 1,
        Stage::Shaping => 2,
        Stage::Holding => 3,
        Stage::Units => 4,
        Stage::Releases => 5,
        Stage::Closing => 6,
    }
}

/// Whether an item of stage `next` may follow the run's current stage.
pub fn ordered(current: &Stage, next: &Stage) -> bool {
    rank(current) <= rank(next)
}

/// One directory step of the host's sub-protocol, for one receipt.
pub enum Action {
    /// Bind a receipt the replaced generation recorded into this run's
    /// journal (a journal record; the namespace is unchanged).
    Bind,
    /// Turn a migrated owned link into the directory tree its receipt
    /// prepares (apply).
    Birth,
    /// Plan a receipt's evolution for this run's leaves, preparing the
    /// private staging names of its new nodes (apply).
    Plan,
    /// Publish a planned evolution (apply).
    Evolve,
    /// Restore a retained receipt's shape over the same root (rollback).
    Shape,
    /// Reverse the current incarnation and reactivate the target's (rollback).
    Exchange,
    /// Reactivate a retained receipt where no directory stands (rollback).
    Reactivate,
    /// The domain holds this destination (an owned node) for the rest of the
    /// run. No effect.
    Hold,
    /// Retire the owned nodes no leaf needs any more (apply).
    Finish,
    /// Reverse the receipt to its predecessor link and settle that link
    /// (apply's undeclare; rollback to a generation without the receipt).
    Retire,
}

/// One directory item, as the host prepared it.
pub struct DirectoryFacts {
    /// The receipt's root (for `Hold`, the held node).
    pub dest: DestId,
    pub action: Action,
}

/// The stage a directory action belongs to.
pub fn stage(action: &Action) -> Stage {
    match action {
        Action::Bind => Stage::Opening,
        Action::Birth => Stage::Opening,
        Action::Plan => Stage::Opening,
        Action::Evolve => Stage::Shaping,
        Action::Shape => Stage::Shaping,
        Action::Exchange => Stage::Shaping,
        Action::Reactivate => Stage::Shaping,
        Action::Hold => Stage::Holding,
        Action::Finish => Stage::Closing,
        Action::Retire => Stage::Closing,
    }
}

/// Whether the action is executed by the host (every action but `Hold`).
pub fn commands(action: &Action) -> bool {
    match action {
        Action::Hold => false,
        Action::Bind => true,
        Action::Birth => true,
        Action::Plan => true,
        Action::Evolve => true,
        Action::Shape => true,
        Action::Exchange => true,
        Action::Reactivate => true,
        Action::Finish => true,
        Action::Retire => true,
    }
}

/// Whether `dest` is in the list (the domain's holds, or rollback's
/// claimed destinations).
pub fn names(dests: &[DestId], dest: DestId) -> bool {
    let mut i = 0;
    while i < dests.len() {
        if dests[i].same(dest) {
            return true;
        }
        i += 1;
    }
    false
}
