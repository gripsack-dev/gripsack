//! GC planning as a pure, verified kernel (0046, handoff §5.3):
//! admission, generation pruning, and deletion-set computation over
//! validated inventory. The effectful collector in
//! `gripsack-exec/src/gc.rs` consumes these plans under a
//! `LifecycleSession`; this module never touches the filesystem.
//!
//! Proved properties:
//!
//! - admission failure produces no destructive plan
//!   (`admit_gc`'s biconditionals),
//! - pruned generations are drawn from the inventory and never name
//!   the current generation (`plan_prune`),
//! - the deletion set is exactly the candidates minus the roots, in
//!   order (`plan_delete`'s extensional contract with `spec_delete`),
//! - growing the root set cannot grow the deletion set
//!   (`lemma_delete_monotone`).

use vstd::prelude::*;
// plain-cargo shim builds see no use of the seq lemmas; the
// `broadcast use` below (erased outside verification) needs them
#[allow(unused_imports)]
use vstd::seq_lib::*;

verus! {

broadcast use group_seq_properties;

/// GC admission (0045 F3, typed in 0046): the destructive command's
/// preconditions as a total function, so a caller cannot forget a
/// check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcAdmission {
    /// Inventory is sound and recovery is finished — plan away.
    Admitted,
    /// A run marker, journal entries, or a quarantine exist:
    /// journaled prior blobs may be referenced by no manifest, so no
    /// deletion set is computable. Reconcile first.
    RecoveryPending,
    /// The current generation has no directory on disk — corruption;
    /// "cannot read" must never read as "nothing referenced".
    CorruptCurrent,
}

/// Admit or refuse destructive collection. Recovery state wins over
/// inventory shape: while recovery is pending, even a sound inventory
/// proves nothing about journal-only blobs.
pub fn admit_gc(recovery_pending: bool, current: Option<u64>, generations: &[u64]) -> (result: GcAdmission)
    ensures
        (result == GcAdmission::RecoveryPending) <==> recovery_pending,
        (result == GcAdmission::CorruptCurrent) <==>
            (!recovery_pending && current.is_some()
            && !generations@.contains(current.unwrap())),
        (result == GcAdmission::Admitted) <==>
            (!recovery_pending && (current.is_none()
            || generations@.contains(current.unwrap()))),
{
    if recovery_pending {
        return GcAdmission::RecoveryPending;
    }
    match current {
        Some(c) if !contains_generation(generations, c) => GcAdmission::CorruptCurrent,
        _ => GcAdmission::Admitted,
    }
}

/// Membership over generation numbers (vstd does not specify
/// `slice::contains`; this loop carries its own contract).
pub fn contains_generation(haystack: &[u64], needle: u64) -> (result: bool)
    ensures
        result == exists|j: int| 0 <= j < haystack.len() && haystack@[j] == needle,
{
    let mut i = 0;
    while i < haystack.len()
        invariant
            i <= haystack.len(),
            forall|j: int| 0 <= j < i ==> haystack@[j] != needle,
        decreases haystack.len() - i,
    {
        if haystack[i] == needle {
            assert(haystack@[i as int] == needle);
            return true;
        }
        i += 1;
    }
    false
}

/// Membership over path identities, with the contract in the form the
/// deletion kernel's spec mirror uses (mapped character views).
pub fn contains_identity(haystack: &[&str], needle: &str) -> (result: bool)
    ensures
        result == haystack@.map(|_i, s: &str| s@).contains(needle@),
{
    let mut i = 0;
    while i < haystack.len()
        invariant
            i <= haystack.len(),
            forall|j: int| 0 <= j < i ==> haystack@[j]@ != needle@,
        decreases haystack.len() - i,
    {
        if haystack[i] == needle {
            assert(haystack@.map(|_i, s: &str| s@)[i as int] == needle@);
            return true;
        }
        i += 1;
    }
    false
}

/// Generation pruning: with `keep` set and more generations on disk
/// than it allows, the oldest `excess` go — EXCEPT the current one,
/// which is never pruned (one extra is kept instead). The inventory
/// arrives sorted ascending (`generations::list`).
pub fn plan_prune(generations: &[u64], current: Option<u64>, keep: Option<u32>) -> (result: Vec<u64>)
    ensures
        // every pruned generation came from the inventory
        forall|g: u64| result@.contains(g) ==> generations@.contains(g),
        // the active generation is never in the deletion set
        current.is_some() ==> !result@.contains(current.unwrap()),
{
    let mut pruned: Vec<u64> = Vec::new();
    if let Some(keep) = keep {
        let n = generations.len();
        let keep = keep as usize;
        if n > keep {
            let excess = n - keep;
            let mut i = 0;
            while i < excess
                invariant
                    i <= excess,
                    excess <= n == generations@.len(),
                    forall|g: u64| pruned@.contains(g) ==>
                        generations@.contains(g)
                        && (current.is_some() ==> g != current.unwrap()),
                decreases excess - i,
            {
                let g = generations[i];
                assert(generations@[i as int] == g);
                if Some(g) != current {
                    pruned.push(g);
                }
                i += 1;
            }
        }
    }
    pruned
}

/// The deletion set: candidates minus roots, order-preserving.
/// `referenced` are the paths retained manifests pin (store payloads,
/// build closures, prior blobs); `candidates` are the objects actually
/// on disk under one inventory directory.
pub fn plan_delete<'a>(referenced: &[&str], candidates: &[&'a str]) -> (result: Vec<&'a str>)
    ensures
        // exactly the spec mirror — subset, disjointness and order
        // properties all read off `spec_delete`'s definition
        result@.map(|_i, s: &str| s@) == spec_delete(
            referenced@.map(|_i, s: &str| s@),
            candidates@.map(|_i, s: &str| s@),
        ),
{
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < candidates.len()
        invariant
            i <= candidates.len(),
            out@.map(|_i, s: &str| s@) == spec_delete_prefix(
                referenced@.map(|_i, s: &str| s@),
                candidates@.map(|_i, s: &str| s@),
                i as int,
            ),
        decreases candidates.len() - i,
    {
        let c = candidates[i];
        if !contains_identity(referenced, c) {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// The spec mirror of [`plan_delete`]: candidates minus roots, in
/// order. Prefix-recursive so the exec loop invariant matches step
/// for step.
pub open spec fn spec_delete(referenced: Seq<Seq<char>>, candidates: Seq<Seq<char>>) -> Seq<Seq<char>> {
    spec_delete_prefix(referenced, candidates, candidates.len() as int)
}

pub open spec fn spec_delete_prefix(
    referenced: Seq<Seq<char>>,
    candidates: Seq<Seq<char>>,
    i: int,
) -> Seq<Seq<char>>
    decreases i,
{
    if i <= 0 {
        Seq::empty()
    } else {
        let rest = spec_delete_prefix(referenced, candidates, i - 1);
        let last = candidates[i - 1];
        if referenced.contains(last) { rest } else { rest.push(last) }
    }
}

/// Monotonicity over prefixes (the induction the main lemma needs).
pub proof fn lemma_delete_monotone_prefix(
    referenced_small: Seq<Seq<char>>,
    referenced_big: Seq<Seq<char>>,
    candidates: Seq<Seq<char>>,
    i: int,
)
    requires
        0 <= i <= candidates.len(),
        forall|x: Seq<char>| referenced_small.contains(x) ==> referenced_big.contains(x),
    ensures
        forall|x: Seq<char>| spec_delete_prefix(referenced_big, candidates, i).contains(x)
            ==> spec_delete_prefix(referenced_small, candidates, i).contains(x),
    decreases i,
{
    if i > 0 {
        lemma_delete_monotone_prefix(referenced_small, referenced_big, candidates, i - 1);
        let last = candidates[i - 1];
        // unfold one recursion step on both sides and case-split on
        // whether the head element is a root
        assert(spec_delete_prefix(referenced_big, candidates, i) ==
            if referenced_big.contains(last) {
                spec_delete_prefix(referenced_big, candidates, i - 1)
            } else {
                spec_delete_prefix(referenced_big, candidates, i - 1).push(last)
            });
        assert(spec_delete_prefix(referenced_small, candidates, i) ==
            if referenced_small.contains(last) {
                spec_delete_prefix(referenced_small, candidates, i - 1)
            } else {
                spec_delete_prefix(referenced_small, candidates, i - 1).push(last)
            });
        // membership after push: the pushed element or the prefix
        assert forall|x: Seq<char>|
            spec_delete_prefix(referenced_big, candidates, i).contains(x) implies
            spec_delete_prefix(referenced_small, candidates, i).contains(x) by {
            if x == last {
            } else {
                assert(spec_delete_prefix(referenced_big, candidates, i - 1).contains(x));
            }
        }
    }
}

/// Monotonicity (handoff §5.3): increasing roots cannot increase the
/// deletable object set for a fixed inventory.
pub proof fn lemma_delete_monotone(
    referenced_small: Seq<Seq<char>>,
    referenced_big: Seq<Seq<char>>,
    candidates: Seq<Seq<char>>,
)
    requires
        forall|x: Seq<char>| referenced_small.contains(x) ==> referenced_big.contains(x),
    ensures
        forall|x: Seq<char>| spec_delete(referenced_big, candidates).contains(x)
            ==> spec_delete(referenced_small, candidates).contains(x),
{
    lemma_delete_monotone_prefix(
        referenced_small,
        referenced_big,
        candidates,
        candidates.len() as int,
    );
}

}
