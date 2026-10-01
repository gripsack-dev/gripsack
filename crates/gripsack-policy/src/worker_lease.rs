//! Managed BuildKit worker admission decisions (plan/0051 B1-02/B1-04).
//!
//! Used by owned-worker identity, acquisition, confirmed release and explicit
//! removal decisions. Docker effects, inherited file-lock leases, sockets and
//! durable records remain separate trusted boundaries. Reconciliation and
//! model-to-runtime correspondence need their own evidence; this is not a proof
//! of Docker or the BuildKit daemon.
use vstd::prelude::*;

verus! {

/// Hard bound on simultaneously live leases of one managed worker. Lease
/// files are bounded host resources; saturation refuses, never queues.
pub const MAX_LIVE_LEASES: u64 = 256;

/// Lifecycle of one owned worker instance, durable in the home record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum WorkerPhase { Provisioning, Ready, Stopping, Stopped, Failed }

/// Result of comparing the durably recorded container identity with the
/// identity currently observed under the worker's name. `Current` is the
/// ONLY identity a destructive action may name; `Stale`/`Unrecorded`
/// retain/quarantine authority for explicit recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum IdentityDecision { Current, Stale, Absent, Unrecorded }

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum AcquireRefusal { NotReady, Saturated }

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum StopRefusal { NotReady, LiveLeases, UncertainWriters }


#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum RemovalRefusal { NotCurrent }

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum ReplacementRefusal { LiveLeases, UncertainWriters, MidTransition }

/// Outcome of comparing a recorded worker's durable image/resource binding
/// with the current request. `Reuse` keeps the warm instance; `Replace`
/// authorizes idle-only removal and recreation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum ReuseDecision { Reuse, Replace }

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum CacheCleanupRefusal { ForeignVolume, WorkerActive, LiveLeases, UncertainWriters }

/// How a released lease is disposed. `Quarantined` keeps stop authority
/// until an explicit owner recovery acknowledges the uncertain writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum ReleaseDisposition { Drained, Quarantined }

/// Pointwise byte equality with an extensional specification: callers
/// compare full immutable identities, never prefixes or names.
pub fn bytes_equal(a: &[u8], b: &[u8]) -> (equal: bool)
    ensures equal == (a@ == b@),
{
    if a.len() != b.len() {
        return false;
    }
    let mut index: usize = 0;
    while index < a.len()
        invariant
            a.len() == b.len(),
            index <= a.len(),
            forall|j: int| 0 <= j < index ==> a@[j] == b@[j],
        decreases a.len() - index,
    {
        if a[index] != b[index] {
            return false;
        }
        index = index + 1;
    }
    assert(a@ =~= b@);
    true
}

/// Classify recorded-vs-observed container identity. Both sides are full
/// immutable Docker IDs (or absent); a mutable name match is NOT identity.
pub fn identity_decision(recorded: Option<&[u8]>, observed: Option<&[u8]>) -> (decision: IdentityDecision)
    ensures
        (decision == IdentityDecision::Current) == (recorded.is_some() && observed.is_some()
            && recorded.unwrap()@ == observed.unwrap()@),
        (decision == IdentityDecision::Stale) == (recorded.is_some() && observed.is_some()
            && recorded.unwrap()@ != observed.unwrap()@),
        (decision == IdentityDecision::Absent) == observed.is_none(),
        (decision == IdentityDecision::Unrecorded) == (recorded.is_none() && observed.is_some()),
{
    match (recorded, observed) {
        (Some(recorded), Some(observed)) => {
            if bytes_equal(recorded, observed) {
                IdentityDecision::Current
            } else {
                IdentityDecision::Stale
            }
        }
        (None, Some(_)) => IdentityDecision::Unrecorded,
        (_, None) => IdentityDecision::Absent,
    }
}

/// A lease is admitted only from a ready, unsaturated worker. Returns the
/// live-lease count after admission so callers cannot drift from it.
pub fn admit_acquire(phase: WorkerPhase, live: u64) -> (admission: Result<u64, AcquireRefusal>)
    ensures
        matches!(admission, Ok(_)) == (phase == WorkerPhase::Ready && live < MAX_LIVE_LEASES),
        match admission { Ok(next) => next == live + 1, Err(_) => true },
        matches!(admission, Err(AcquireRefusal::Saturated)) ==> live >= MAX_LIVE_LEASES,
{
    if phase != WorkerPhase::Ready {
        Err(AcquireRefusal::NotReady)
    } else if live >= MAX_LIVE_LEASES {
        Err(AcquireRefusal::Saturated)
    } else {
        Ok(live + 1)
    }
}

/// THE stop fence (Epic B §10.3): an owned stop is admitted only on an
/// idle ready worker — zero live leases AND zero uncertain/quarantined
/// writers. A stop on the last lease is legal only once that lease's
/// cleanup was confirmed; anything uncertain retains authority.
pub fn admit_stop(phase: WorkerPhase, live: u64, uncertain: u64) -> (admission: Result<(), StopRefusal>)
    ensures
        matches!(admission, Ok(_)) == (phase == WorkerPhase::Ready && live == 0 && uncertain == 0),
        matches!(admission, Err(StopRefusal::LiveLeases)) ==> live > 0,
        matches!(admission, Err(StopRefusal::UncertainWriters)) ==> live == 0 && uncertain > 0,
{
    if phase != WorkerPhase::Ready {
        Err(StopRefusal::NotReady)
    } else if live > 0 {
        Err(StopRefusal::LiveLeases)
    } else if uncertain > 0 {
        Err(StopRefusal::UncertainWriters)
    } else {
        Ok(())
    }
}


/// A destructive container removal may name ONLY the durably recorded
/// immutable identity. Anything else — a stale observed ID or a container
/// nobody recorded — keeps its authority; the caller quarantines/recovers.
pub fn admit_removal(identity: IdentityDecision) -> (admission: Result<(), RemovalRefusal>)
    ensures
        matches!(admission, Ok(_)) == (identity == IdentityDecision::Current),
        matches!(admission, Err(_)) ==> identity != IdentityDecision::Current,
{
    if identity == IdentityDecision::Current {
        Ok(())
    } else {
        Err(RemovalRefusal::NotCurrent)
    }
}

/// Idle-only resource/version replacement (Epic B §4.2): a recorded worker
/// whose durable image or resource binding differs from the current request
/// may be replaced ONLY with zero live leases, zero uncertain writers and a
/// settled (ready/stopped) phase — an upgrade or resource change with live
/// clients is refused, never queued or forced. A matching binding always
/// reuses the warm instance.
pub fn admit_replacement(matching: bool, phase: WorkerPhase, live: u64, uncertain: u64) -> (admission: Result<ReuseDecision, ReplacementRefusal>)
    ensures
        matches!(admission, Ok(ReuseDecision::Reuse)) == matching,
        matches!(admission, Ok(ReuseDecision::Replace)) == (!matching && live == 0 && uncertain == 0
            && (phase == WorkerPhase::Ready || phase == WorkerPhase::Stopped)),
        matches!(admission, Err(ReplacementRefusal::LiveLeases)) ==> live > 0,
        matches!(admission, Err(ReplacementRefusal::UncertainWriters)) ==> live == 0 && uncertain > 0,
        matches!(admission, Err(ReplacementRefusal::MidTransition)) ==> live == 0 && uncertain == 0
            && phase != WorkerPhase::Ready && phase != WorkerPhase::Stopped,
{
    if matching {
        Ok(ReuseDecision::Reuse)
    } else if live > 0 {
        Err(ReplacementRefusal::LiveLeases)
    } else if uncertain > 0 {
        Err(ReplacementRefusal::UncertainWriters)
    } else if phase != WorkerPhase::Ready && phase != WorkerPhase::Stopped {
        Err(ReplacementRefusal::MidTransition)
    } else {
        Ok(ReuseDecision::Replace)
    }
}

/// Disposition of one released lease. The last-confirmed-lease rule: only
/// a confirmed cleanup drains; an uncertain outcome quarantines the writer
/// so no later stop or replacement can free the worker underneath it.
pub fn release_disposition(cleanup_confirmed: bool) -> (disposition: ReleaseDisposition)
    ensures
        (disposition == ReleaseDisposition::Drained) == cleanup_confirmed,
        (disposition == ReleaseDisposition::Quarantined) == !cleanup_confirmed,
{
    if cleanup_confirmed {
        ReleaseDisposition::Drained
    } else {
        ReleaseDisposition::Quarantined
    }
}

/// Explicit cache-volume cleanup: only for a stopped worker with no live
/// or uncertain writers and a volume whose ownership labels verified. The
/// cache is separate from retained artifacts and is never swept on stop.
pub fn admit_cache_cleanup(
    phase: WorkerPhase,
    live: u64,
    uncertain: u64,
    volume_owned: bool,
) -> (admission: Result<(), CacheCleanupRefusal>)
    ensures
        matches!(admission, Ok(_)) == (phase == WorkerPhase::Stopped && live == 0 && uncertain == 0 && volume_owned),
        matches!(admission, Err(CacheCleanupRefusal::ForeignVolume)) ==> !volume_owned,
        matches!(admission, Err(CacheCleanupRefusal::WorkerActive)) ==> volume_owned && phase != WorkerPhase::Stopped,
{
    if !volume_owned {
        Err(CacheCleanupRefusal::ForeignVolume)
    } else if phase != WorkerPhase::Stopped {
        Err(CacheCleanupRefusal::WorkerActive)
    } else if live > 0 {
        Err(CacheCleanupRefusal::LiveLeases)
    } else if uncertain > 0 {
        Err(CacheCleanupRefusal::UncertainWriters)
    } else {
        Ok(())
    }
}

/// A confirmed stop retires only earlier epochs of the same owned namespace.
/// The effect adapter owns the stop receipt and the exclusive native root lock.
pub fn retired_epoch(same_namespace: bool, epoch: u64, next_epoch: u64) -> (retired: bool)
    ensures retired == (same_namespace && 0 < epoch && epoch < next_epoch),
{
    same_namespace && 0 < epoch && epoch < next_epoch
}

}
