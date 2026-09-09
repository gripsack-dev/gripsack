//! Ownership decisions (0029, 0033 R7, 0046): the copy/link authority
//! kernels. Pure functions over typed identities; the lineage explorer
//! drives these same functions through materialized filesystem states,
//! and the verifier proves the authority table over all inputs.
//!
//! The contracts below are the AUTHORITY RULES, not the branch
//! structure (handoff §5.2): preserved drift never authorizes; a
//! managed update requires agreement with the last managed write;
//! explicit take-over always absorbs; reconvergence (live == desired)
//! is the only way back from drift.

use vstd::prelude::*;

verus! {

/// The copy/template disposition as a pure function's output (0033 R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyPlan {
    /// Nothing live: create.
    Fresh,
    /// Live IS the desired content.
    Satisfied,
    /// Live is what gripsack last wrote (managed): authorized update.
    Update,
    /// Live is foreign or drifted: preserve and report.
    Preserve,
    /// Explicit user consent to absorb whatever is live.
    TakeOver,
}

/// The tracked-copy/template decision. `desired` and `live` are the
/// manifest-domain identities (mode-aware file identities; a foreign
/// link hashes its target); `prev` is the lineage record — last
/// managed write and whether it was preserved drift, which authorizes
/// nothing. Specs speak in `@` (Seq<char>) views: extensional content
/// equality, the same equality the exec `==` implements.
pub fn plan_copy(
    desired: &str,
    live: Option<&str>,
    prev: Option<(&str, bool)>,
    take_over: bool,
) -> (result: CopyPlan)
    ensures
        // nothing live means create, whatever the lineage says
        (result == CopyPlan::Fresh) <==> live.is_none(),
        // explicit consent/absorb ALWAYS captures the origin — even
        // when the bytes already match (adopt relies on this to open
        // the epoch)
        (result == CopyPlan::TakeOver) <==> (take_over && live.is_some()),
        // live IS the desired content (reconvergence) and no take-over
        // was requested
        (result == CopyPlan::Satisfied) <==>
            (!take_over && live.is_some() && live.unwrap()@ == desired@),
        // managed, and live is our last write: the clean update path
        (result == CopyPlan::Update) <==>
            (!take_over && live.is_some() && live.unwrap()@ != desired@
            && prev.is_some() && !prev.unwrap().1
            && live.unwrap()@ == prev.unwrap().0@),
        // preserved drift never authorizes — only reconvergence
        // (handled above) ends the drift state
        (result == CopyPlan::Preserve) <==>
            (!take_over && live.is_some() && live.unwrap()@ != desired@
            && !(prev.is_some() && !prev.unwrap().1
                && live.unwrap()@ == prev.unwrap().0@)),
{
    let Some(live) = live else {
        return CopyPlan::Fresh;
    };
    if take_over {
        return CopyPlan::TakeOver;
    }
    if live == desired {
        return CopyPlan::Satisfied;
    }
    match prev {
        // managed and live is our last write: the clean update path
        // (never a fresh take-over — the epoch's origin stands)
        Some((written, false)) if live == written => CopyPlan::Update,
        // explicit consent outranks preservation: --take-over absorbs
        // whatever is live and begins a new epoch with it as origin
        _ if take_over => CopyPlan::TakeOver,
        _ => CopyPlan::Preserve,
    }
}

/// The owned-link disposition (0033 R7). `exists`: anything at the
/// destination; `ours`: a link into the store; `recorded`: a previous
/// manifest entry that is NOT preserved drift (preserved drift
/// authorizes nothing, including a mode change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkPlan {
    /// Nothing there (or already ours to redeploy): point the link.
    Link,
    /// Absorb the foreign object --take-over (a prior is captured).
    TakeOver,
    /// A foreign object blocks the deploy — move it or --take-over.
    Refuse,
}

pub fn plan_link(exists: bool, ours: bool, recorded: bool, take_over: bool) -> (result: LinkPlan)
    ensures
        // a foreign, unrecorded object blocks without explicit consent
        (result == LinkPlan::Refuse) <==> (exists && !ours && !recorded && !take_over),
        // consent absorbs the foreign object — never one that is ours
        // or already recorded (those are plain re-links)
        (result == LinkPlan::TakeOver) <==> (take_over && !ours && !recorded),
        // everything else is a plain (re-)point: nothing there, our
        // own link, or a recorded destination
        (result == LinkPlan::Link) <==> (ours || recorded || (!exists && !take_over)),
{
    if exists && !ours && !recorded && !take_over {
        LinkPlan::Refuse
    } else if take_over && !ours && !recorded {
        LinkPlan::TakeOver
    } else {
        LinkPlan::Link
    }
}

}
