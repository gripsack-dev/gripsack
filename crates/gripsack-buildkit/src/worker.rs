//! Managed worker lease transitions (Epic B §4/§10.3, plan/0051 B1).
//! Pure kernel only: the invariant the backend must never break is
//! **an instance cannot stop, upgrade or be deleted while a live lease
//! requires it**, and cleanup decisions are made from this table, never
//! ad hoc at an effect site. The TLC model (`specs/WorkerLease.tla`)
//! checks the same transitions under two-client interleavings and
//! crashes; this kernel is what production calls. Effectful
//! provisioning (containers, VMs, sockets) lands with B1-01 and calls
//! these decisions.

use std::collections::BTreeMap;

/// Worker instance lifecycle. `Failed` is reachable from any live
/// state (the daemon can die); there is deliberately no automatic
/// restart inside the kernel — recovery is an explicit owner action
/// so a crashed worker can never be silently reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Provisioning,
    Ready,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LeaseId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientId(u64);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("worker is {0:?}; leases are only acquired from a ready worker")]
    NotReady(WorkerState),
    #[error("worker is {0:?}; leases are only released from a ready or stopping worker")]
    NotLeasable(WorkerState),
    #[error("lease {0:?} is unknown to this worker")]
    UnknownLease(LeaseId),
    #[error("cannot stop a worker with {0} live lease(s); drain them first")]
    LiveLeases(usize),
    #[error("cannot transition a {0:?} worker to {1:?}")]
    IllegalTransition(WorkerState, WorkerState),
}

/// One managed worker's lease table. A lease is "live" from acquire
/// until its release; a worker may hold leases from many clients at
/// once (two real clients is the B1-02 acceptance case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLeases {
    state: WorkerState,
    next_lease: u64,
    leases: BTreeMap<LeaseId, ClientId>,
}

impl WorkerLeases {
    /// A worker begins provisioning: no leases may exist yet.
    pub fn provisioning() -> Self {
        Self {
            state: WorkerState::Provisioning,
            next_lease: 1,
            leases: BTreeMap::new(),
        }
    }

    pub fn state(&self) -> WorkerState {
        self.state
    }

    pub fn live_leases(&self) -> usize {
        self.leases.len()
    }

    pub fn lease_holders(&self) -> impl Iterator<Item = (LeaseId, ClientId)> + '_ {
        self.leases.iter().map(|(lease, client)| (*lease, *client))
    }

    /// Bootstrap finished successfully.
    pub fn provisioned(&mut self) -> Result<(), LeaseError> {
        self.transition(WorkerState::Ready)
    }

    /// The daemon/VM died. Leases do NOT disappear silently: the table
    /// keeps them so `drain` remains explicit and observable.
    pub fn failed(&mut self) -> Result<(), LeaseError> {
        match self.state {
            WorkerState::Stopped | WorkerState::Failed => Err(LeaseError::IllegalTransition(
                self.state,
                WorkerState::Failed,
            )),
            _ => {
                self.state = WorkerState::Failed;
                Ok(())
            }
        }
    }

    /// Acquire a lease for a client. Only a ready worker leases.
    pub fn acquire(&mut self, client: ClientId) -> Result<LeaseId, LeaseError> {
        if self.state != WorkerState::Ready {
            return Err(LeaseError::NotReady(self.state));
        }
        let lease = LeaseId(self.next_lease);
        self.next_lease += 1;
        self.leases.insert(lease, client);
        Ok(lease)
    }

    /// Release a previously acquired lease.
    pub fn release(&mut self, lease: LeaseId) -> Result<ClientId, LeaseError> {
        if !matches!(self.state, WorkerState::Ready | WorkerState::Stopping) {
            return Err(LeaseError::NotLeasable(self.state));
        }
        self.leases
            .remove(&lease)
            .ok_or(LeaseError::UnknownLease(lease))
    }

    /// Idle-only stop: rejected while any live lease requires the
    /// worker. This is THE kernel invariant (Epic B §10.3: "Do not
    /// release ... a worker lease while an owned writer may still use
    /// it" — the stop-side dual).
    pub fn request_stop(&mut self) -> Result<(), LeaseError> {
        match self.state {
            WorkerState::Ready if self.leases.is_empty() => {
                self.state = WorkerState::Stopping;
                Ok(())
            }
            WorkerState::Ready => Err(LeaseError::LiveLeases(self.leases.len())),
            WorkerState::Stopping | WorkerState::Stopped => Err(LeaseError::IllegalTransition(
                self.state,
                WorkerState::Stopping,
            )),
            other => Err(LeaseError::IllegalTransition(other, WorkerState::Stopping)),
        }
    }

    /// The stop completed: only legal from Stopping (which itself is
    /// only reachable with zero live leases), so a stopped worker
    /// never abandoned a lease.
    pub fn stopped(&mut self) -> Result<(), LeaseError> {
        match self.state {
            WorkerState::Stopping if self.leases.is_empty() => {
                self.state = WorkerState::Stopped;
                Ok(())
            }
            WorkerState::Stopping => Err(LeaseError::LiveLeases(self.leases.len())),
            other => Err(LeaseError::IllegalTransition(other, WorkerState::Stopped)),
        }
    }

    /// Explicit owner recovery of a crashed worker. Requires every
    /// lease to have been drained first: a stale lease on a failed
    /// worker must be resolved by its holder (bounded diagnostics),
    /// never wiped by the recovery path.
    pub fn recover(&mut self) -> Result<(), LeaseError> {
        match self.state {
            WorkerState::Failed if self.leases.is_empty() => {
                self.state = WorkerState::Provisioning;
                Ok(())
            }
            WorkerState::Failed => Err(LeaseError::LiveLeases(self.leases.len())),
            other => Err(LeaseError::IllegalTransition(
                other,
                WorkerState::Provisioning,
            )),
        }
    }

    fn transition(&mut self, to: WorkerState) -> Result<(), LeaseError> {
        let legal = matches!(
            (self.state, to),
            (WorkerState::Provisioning, WorkerState::Ready)
                | (WorkerState::Provisioning, WorkerState::Failed)
                | (WorkerState::Ready, WorkerState::Failed)
                | (WorkerState::Stopping, WorkerState::Failed)
        );
        if legal {
            self.state = to;
            Ok(())
        } else {
            Err(LeaseError::IllegalTransition(self.state, to))
        }
    }
}

/// Owned-resource cleanup guard: a cleanup decision names exactly the
/// resources this manager created (`owned`), so a user-supplied worker
/// or a retained host artifact can never be swept by mistake (the
/// B1-02 "owned cleanup leaves user-supplied workers and retained host
/// artifacts intact" acceptance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupSet {
    owned: std::collections::BTreeSet<String>,
    retained: std::collections::BTreeSet<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CleanupError {
    #[error("{0:?} is not owned by this manager")]
    NotOwned(String),
    #[error("{0:?} is retained and can never be swept")]
    Retained(String),
}

impl CleanupSet {
    pub fn new() -> Self {
        Self {
            owned: Default::default(),
            retained: Default::default(),
        }
    }

    pub fn own(&mut self, resource: impl Into<String>) {
        self.owned.insert(resource.into());
    }

    pub fn retain(&mut self, resource: impl Into<String>) {
        self.retained.insert(resource.into());
    }

    /// Decide whether `resource` may be deleted. Only explicit
    /// `own`ed names delete; retained names never do.
    pub fn may_delete(&self, resource: &str) -> Result<bool, CleanupError> {
        if self.retained.contains(resource) {
            return Err(CleanupError::Retained(resource.to_owned()));
        }
        Ok(self.owned.contains(resource))
    }
}

impl Default for CleanupSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ClientId = ClientId(1);
    const B: ClientId = ClientId(2);

    #[test]
    fn two_clients_lease_one_ready_worker_concurrently() {
        let mut worker = WorkerLeases::provisioning();
        worker.provisioned().unwrap();
        let first = worker.acquire(A).unwrap();
        let second = worker.acquire(B).unwrap();
        assert_ne!(first, second);
        assert_eq!(worker.live_leases(), 2);
        assert_eq!(
            worker.lease_holders().collect::<Vec<_>>(),
            vec![(first, A), (second, B)]
        );
        // Both drain, then the idle stop is legal.
        worker.release(first).unwrap();
        worker.release(second).unwrap();
        worker.request_stop().unwrap();
        worker.stopped().unwrap();
    }

    #[test]
    fn a_worker_with_a_live_lease_cannot_stop_or_be_deleted() {
        let mut worker = WorkerLeases::provisioning();
        worker.provisioned().unwrap();
        let lease = worker.acquire(A).unwrap();
        assert_eq!(
            worker.request_stop(),
            Err(LeaseError::LiveLeases(1)),
            "idle-only replacement: the live lease blocks the stop"
        );
        // The holder drains; only then may the stop proceed.
        worker.release(lease).unwrap();
        worker.request_stop().unwrap();
    }

    #[test]
    fn a_crashed_worker_keeps_its_leases_until_explicit_drain() {
        let mut worker = WorkerLeases::provisioning();
        worker.provisioned().unwrap();
        let lease = worker.acquire(A).unwrap();
        worker.failed().unwrap();
        assert_eq!(worker.state(), WorkerState::Failed);
        // Leases did not silently vanish.
        assert_eq!(worker.live_leases(), 1);
        // Recovery is blocked until the stale lease is resolved.
        assert_eq!(worker.recover(), Err(LeaseError::LiveLeases(1)));
        // The holder resolves it on the failed worker...
        assert_eq!(
            worker.release(lease),
            Err(LeaseError::NotLeasable(WorkerState::Failed))
        );
        // ...via the explicit drain path, modeled here as the owner
        // acknowledging the stale lease after bounded diagnostics.
        worker.leases.remove(&lease);
        worker.recover().unwrap();
        assert_eq!(worker.state(), WorkerState::Provisioning);
        worker.provisioned().unwrap();
    }

    #[test]
    fn illegal_transitions_reject_instead_of_guessing() {
        let mut worker = WorkerLeases::provisioning();
        assert_eq!(
            worker.stopped(),
            Err(LeaseError::IllegalTransition(
                WorkerState::Provisioning,
                WorkerState::Stopped
            ))
        );
        assert_eq!(
            worker.acquire(A),
            Err(LeaseError::NotReady(WorkerState::Provisioning))
        );
        worker.provisioned().unwrap();
        assert_eq!(
            worker.provisioned(),
            Err(LeaseError::IllegalTransition(
                WorkerState::Ready,
                WorkerState::Ready
            ))
        );
        worker.request_stop().unwrap();
        let mut dead = WorkerLeases::provisioning();
        dead.failed().unwrap();
        assert_eq!(
            dead.failed(),
            Err(LeaseError::IllegalTransition(
                WorkerState::Failed,
                WorkerState::Failed
            ))
        );
        assert_eq!(
            dead.request_stop(),
            Err(LeaseError::IllegalTransition(
                WorkerState::Failed,
                WorkerState::Stopping
            ))
        );
    }

    #[test]
    fn unknown_leases_reject_and_ids_never_repeat() {
        let mut worker = WorkerLeases::provisioning();
        worker.provisioned().unwrap();
        assert_eq!(
            worker.release(LeaseId(99)),
            Err(LeaseError::UnknownLease(LeaseId(99)))
        );
        let first = worker.acquire(A).unwrap();
        worker.release(first).unwrap();
        let second = worker.acquire(A).unwrap();
        assert_ne!(first, second, "a released lease id is never reused");
    }

    #[test]
    fn cleanup_only_deletes_owned_resources_and_never_retained_ones() {
        let mut cleanup = CleanupSet::new();
        cleanup.own("gripsack-worker-1");
        cleanup.retain("store/packages");
        assert!(cleanup.may_delete("gripsack-worker-1").unwrap());
        assert!(!cleanup.may_delete("user-supplied-buildkitd").unwrap());
        assert_eq!(
            cleanup.may_delete("store/packages"),
            Err(CleanupError::Retained("store/packages".into()))
        );
    }
}
