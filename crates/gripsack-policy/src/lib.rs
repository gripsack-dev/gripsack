//! gripsack decision kernels (plan/0046): the small, pure policy
//! functions whose correctness the whole transaction protocol leans
//! on, in one dependency-light crate so ONE implementation serves
//! production, the Rust explorers, and the verifier.
//!
//! Verification: `cargo verus verify -p gripsack-policy` (the compose
//! `verify` gate, scripts/check_verus.sh) proves the contracts. Plain
//! `cargo build` compiles the same source with specifications erased,
//! so the musl release and every ordinary build need no verifier.
//!
//! Crate rules (handoff §6.1): no filesystem, network, subprocess,
//! tracing, or async dependencies. Effects stay in the caller's crate;
//! rendering stays out of the kernels.
use vstd::prelude::*;

pub mod ownership;
pub mod retention;

verus! {

/// The commit classification of an interrupted run (0019, 0026 §4,
/// 0028): recovery compares the run marker's declared transaction
/// against the live `current` pointer by EXACT identity. Numeric
/// ordering never decides — a crashed roll-forward has
/// current < target before the flip and is not committed by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// The flip landed: the generation owns the truth; journal
    /// cleanup only.
    Committed,
    /// The flip never landed: restore every journaled prior.
    Uncommitted,
    /// Neither: corruption or tampering — recovery changes nothing
    /// and keeps the journal intact (fail closed).
    Ambiguous,
}

/// The classifier's whole input, named: the facts a run marker
/// carries, plus the live `current` read at recovery time. (A struct,
/// not positional `Option<u64>`-flavored arguments.)
#[derive(Debug, Clone, Copy)]
pub struct RecoveryFacts {
    /// The generation the run started from — None is a fresh
    /// machine's first run.
    pub previous: Option<u64>,
    /// The generation the run was building toward.
    pub target: u64,
    /// `current` on disk when recovery ran.
    pub current: Option<u64>,
}

/// The commit classifier as a pure function (0028), exact equality
/// only. The ensures clauses ARE the specification — derived from the
/// commit rule, not the branch structure: target precedence on an
/// equal previous/target is stated explicitly (handoff 5.1: never
/// silently assume distinctness), and the absence of any inequality
/// in the postconditions is the machine-checked form of "numeric
/// ordering and Apply/Rollback labels do not decide commitment".
pub fn classify(facts: &RecoveryFacts) -> (result: Classification)
    ensures
        // committed exactly when current sits at the target
        (result == Classification::Committed) <==> facts.current == Some(facts.target),
        // uncommitted exactly on a fresh machine with no current, or
        // current back at the previous generation (and NOT also the
        // target — target precedence, 0045)
        (result == Classification::Uncommitted) <==> (
            (facts.current.is_none() && facts.previous.is_none())
            || (facts.previous.is_some() && facts.current == facts.previous
                && facts.current != Some(facts.target))
        ),
        // everything else blocks
        (result == Classification::Ambiguous) <==> (
            facts.current != Some(facts.target)
            && !(facts.current.is_none() && facts.previous.is_none())
            && !(facts.previous.is_some() && facts.current == facts.previous)
        ),
{
    match (facts.previous, facts.current) {
        (Some(_), Some(c)) if c == facts.target => Classification::Committed,
        (Some(prev), Some(c)) if c == prev => Classification::Uncommitted,
        (Some(_), _) => Classification::Ambiguous,
        // a fresh machine's first run: current at the target means the
        // flip landed; absent means it never did
        (None, Some(c)) if c == facts.target => Classification::Committed,
        (None, None) => Classification::Uncommitted,
        (None, Some(_)) => Classification::Ambiguous,
    }
}

}
