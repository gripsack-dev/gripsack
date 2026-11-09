//! GC retention as a pure kernel (0046 §5.3, 0056 K4): admission, generation
//! pruning, and the deletion set over an admitted inventory. The effectful
//! collector in `gripsack-exec/src/gc.rs` consumes these plans under a
//! `LifecycleSession`; this module never touches the filesystem.
//!
//! Proved (proofs/Kernel/Retention.lean):
//!
//! - nothing is planned while recovery or activation is pending, and a
//!   current generation missing from the inventory is refused
//!   (`admit_gc`'s biconditionals);
//! - pruned generations are the oldest excess of the inventory, never the
//!   current generation (`plan_prune`);
//! - the deletion set is exactly the candidates minus the roots, by position
//!   and in order (`plan_delete`).
use crate::{GenerationId, GenerationList, generation::names_generation};
use alloc::vec::Vec;

/// GC admission (0045 F3): the destructive command's preconditions as a total
/// function, so a caller cannot forget a check.
pub enum GcAdmission {
    /// Inventory is sound and recovery is finished — plan away.
    Admitted,
    /// A run marker, journal entries, a quarantine or a pending activation
    /// exist: journaled prior blobs may be referenced by no manifest, so no
    /// deletion set is computable. Reconcile first.
    RecoveryPending,
    /// The current generation has no directory on disk — corruption;
    /// "cannot read" must never read as "nothing referenced".
    CorruptCurrent,
}

/// Admit or refuse destructive collection. Recovery state wins over inventory
/// shape: while recovery is pending, even a sound inventory proves nothing
/// about journal-only blobs.
pub fn admit_gc(
    recovery_pending: bool,
    current: Option<GenerationId>,
    inventory: &GenerationList,
) -> GcAdmission {
    if recovery_pending {
        return GcAdmission::RecoveryPending;
    }
    match current {
        Some(current) => {
            if contains_generation(inventory.as_slice(), current) {
                GcAdmission::Admitted
            } else {
                GcAdmission::CorruptCurrent
            }
        }
        None => GcAdmission::Admitted,
    }
}

/// Membership over generation identities.
pub fn contains_generation(haystack: &[GenerationId], needle: GenerationId) -> bool {
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i].same(needle) {
            return true;
        }
        i += 1;
    }
    false
}

/// Prune exactly the oldest excess prefix except current. Current protection
/// can retain one extra generation; it never shifts deletion to a newer one.
/// Inventory admission has already checked strict ascending order.
pub fn plan_prune(
    inventory: &GenerationList,
    current: Option<GenerationId>,
    keep: Option<usize>,
) -> Vec<GenerationId> {
    let generations = inventory.as_slice();
    let excess = match keep {
        Some(keep) if generations.len() > keep => generations.len() - keep,
        _ => 0,
    };
    let mut pruned: Vec<GenerationId> = Vec::new();
    let mut i = 0;
    while i < excess {
        let candidate = generations[i];
        if !names_generation(current, candidate) {
            pruned.push(candidate);
        }
        i += 1;
    }
    pruned
}

/// Byte-exact identity of two path identities.
pub fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Membership over path identities. Identities are owned byte vectors: a
/// slice of borrowed slices is a nested borrow, which the Aeneas
/// translation does not support.
pub fn contains_bytes(haystack: &[Vec<u8>], needle: &[u8]) -> bool {
    let mut i = 0;
    while i < haystack.len() {
        if same_bytes(&haystack[i], needle) {
            return true;
        }
        i += 1;
    }
    false
}

/// The deletion set: the positions of the candidates that no root names, in
/// order. `referenced` are the paths retained manifests pin (store payloads,
/// build closures, prior blobs); `candidates` are the objects actually on
/// disk under one inventory directory.
pub fn plan_delete(referenced: &[Vec<u8>], candidates: &[Vec<u8>]) -> Vec<usize> {
    let mut deletions: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < candidates.len() {
        if !contains_bytes(referenced, &candidates[i]) {
            deletions.push(i);
        }
        i += 1;
    }
    deletions
}
