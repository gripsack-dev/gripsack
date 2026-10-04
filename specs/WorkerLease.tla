--------------------------- MODULE WorkerLease ---------------------------
(***************************************************************************
B1 owned-worker lifecycle safety (Epic B 4.2/10.3, plan 0051 B1 fences):
a managed worker may not stop while a live lease requires it, leases never
leak into a stopped or re-provisioned worker, a crash never silently erases
leases, recovery drains stale lease evidence before re-provisioning, durable
lease/epoch counters never reset, and a stop (quiescence) receipt retires
only earlier-epoch build roots of the SAME owner incarnation — a restarted
record (new owner) or a later start (new epoch) is never retired by an old
receipt. Two clients interleave; the environment may crash the worker in
any live moment and the durable namespace record may be reincarnated.

Mirrors the production transitions in gripsack_buildkit::worker::manager
(acquire/release/reconcile_stop/stop) and home (durable record/owner
marker): owner  ~ InstanceRecord.owner nonce (reincarnated only on record
recreation), epoch/nextEpoch ~ record.epoch/next_epoch, nextLease ~
record.next_lease, leases ~ the lease flock inventory, receipts ~
WorkerQuiescence, roots ~ retained build roots, RetireRoot ~
WorkerQuiescence::retires / worker_lease::retired_epoch. Provisioning
effects, sockets, containers, VMs and timing are NOT modeled.

The mutant switches (StopIgnoresLeases, CrashWipesLeases,
RetireIgnoresOwner, RetireIgnoresEpoch) exist ONLY in the calibrated
negative configs and are FALSE in the positive one.
***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS TwoClients, CrashEnabled,
          StopIgnoresLeases, CrashWipesLeases, RetireIgnoresOwner, RetireIgnoresEpoch
Clients == {1, 2}
States == {"provisioning", "ready", "stopping", "stopped", "failed"}
MaxEpoch == 3
MaxLeaseId == 3
MaxOwner == 2

LeaseSpace == [id: 1..MaxLeaseId, client: Clients, owner: 1..MaxOwner, epoch: 1..MaxEpoch]
RootSpace == [owner: 1..MaxOwner, epoch: 1..MaxEpoch]
ReceiptSpace == [owner: 1..MaxOwner, fence: 1..(MaxEpoch + 1)]

VARIABLES phase, owner, epoch, nextEpoch, leases, nextLease,
          roots, receipts, retired, vanished
variables == <<phase, owner, epoch, nextEpoch, leases, nextLease,
               roots, receipts, retired, vanished>>

LeaseFree == leases = {}

(***************************************************************************
Lifecycle. StartWorker assigns the durable epoch fence and bumps the
monotone counter; the receipt fences stop authority for later starts.
***************************************************************************)
StartWorker ==
    /\ phase = "provisioning"
    /\ nextEpoch <= MaxEpoch
    /\ epoch' = nextEpoch
    /\ nextEpoch' = nextEpoch + 1
    /\ phase' = "ready"
    /\ UNCHANGED <<owner, leases, nextLease, roots, receipts, retired, vanished>>

Acquire(c) ==
    /\ phase = "ready"
    /\ nextLease <= MaxLeaseId
    /\ leases' = leases \cup
        {[id |-> nextLease, client |-> c, owner |-> owner, epoch |-> epoch]}
    /\ nextLease' = nextLease + 1
    /\ UNCHANGED <<phase, owner, epoch, nextEpoch, roots, receipts, retired, vanished>>

\* Confirmed cleanup drains the lease even after a crash quarantine.
Release(c) ==
    /\ phase \in {"ready", "stopping", "failed"}
    /\ \E l \in leases : l.client = c
    /\ leases' = leases \ {x \in leases : x.client = c}
    /\ UNCHANGED <<phase, owner, epoch, nextEpoch, nextLease,
                   roots, receipts, retired, vanished>>

\* Unconfirmed cleanup quarantines: the lease evidence is kept until drain.
Quarantine(c) ==
    /\ phase = "ready"
    /\ \E l \in leases : l.client = c
    /\ phase' = "failed"
    /\ UNCHANGED <<owner, epoch, nextEpoch, leases, nextLease,
                   roots, receipts, retired, vanished>>

\* Owner recovery drains stale lease evidence one writer at a time.
DrainStale ==
    /\ phase = "failed"
    /\ ~LeaseFree
    /\ \E l \in leases : leases' = leases \ {l}
    /\ UNCHANGED <<phase, owner, epoch, nextEpoch, nextLease,
                   roots, receipts, retired, vanished>>

RequestStop ==
    /\ phase = "ready"
    /\ LeaseFree \/ StopIgnoresLeases
    /\ phase' = "stopping"
    /\ UNCHANGED <<owner, epoch, nextEpoch, leases, nextLease,
                   roots, receipts, retired, vanished>>

StopDone ==
    /\ phase = "stopping"
    /\ LeaseFree
    /\ phase' = "stopped"
    /\ receipts' = receipts \cup {[owner |-> owner, fence |-> nextEpoch]}
    /\ UNCHANGED <<owner, epoch, nextEpoch, leases, nextLease, roots, retired, vanished>>

\* An explicit stop also reconciles a drained failed worker (quiescence).
ReconcileStop ==
    /\ phase = "failed"
    /\ LeaseFree
    /\ phase' = "stopped"
    /\ receipts' = receipts \cup {[owner |-> owner, fence |-> nextEpoch]}
    /\ UNCHANGED <<owner, epoch, nextEpoch, leases, nextLease, roots, retired, vanished>>

Crash ==
    /\ CrashEnabled
    /\ phase \in {"provisioning", "ready", "stopping"}
    /\ phase' = "failed"
    /\ IF CrashWipesLeases /\ ~LeaseFree
       THEN /\ leases' = {} /\ vanished' = TRUE
       ELSE /\ leases' = leases /\ vanished' = vanished
    /\ UNCHANGED <<owner, epoch, nextEpoch, nextLease,
                   roots, receipts, retired>>

\* Same-incarnation recovery: drained failure back to provisioning; the
\* durable counters and receipts are untouched.
Reprovision ==
    /\ phase = "failed"
    /\ LeaseFree
    /\ phase' = "provisioning"
    /\ UNCHANGED <<owner, epoch, nextEpoch, leases, nextLease,
                   roots, receipts, retired, vanished>>

\* Durable record recreation: a NEW owner incarnation. Counters restart
\* only under the new owner; old receipts stay scoped to the old owner.
Reincarnate ==
    /\ phase \in {"failed", "stopped"}
    /\ LeaseFree
    /\ owner < MaxOwner
    /\ owner' = owner + 1
    /\ epoch' = 0
    /\ nextEpoch' = 1
    /\ phase' = "provisioning"
    /\ UNCHANGED <<leases, nextLease, roots, receipts, retired, vanished>>

\* Same-incarnation restart: acquire on a stopped worker re-provisions and
\* re-starts it under the SAME owner with the next durable epoch.
Restart ==
    /\ phase = "stopped"
    /\ phase' = "provisioning"
    /\ UNCHANGED <<owner, epoch, nextEpoch, leases, nextLease,
                   roots, receipts, retired, vanished>>

\* A build publishes a retained root bound to the current incarnation and
\* epoch; publication happens under a live lease of that exact binding.
PublishRoot ==
    /\ phase = "ready"
    /\ ~LeaseFree
    /\ [owner |-> owner, epoch |-> epoch] \notin roots
    /\ roots' = roots \cup {[owner |-> owner, epoch |-> epoch]}
    /\ UNCHANGED <<phase, owner, epoch, nextEpoch, leases, nextLease,
                   receipts, retired, vanished>>

\* Recovery retirement (worker_lease::retired_epoch): a stop receipt of the
\* SAME owner incarnation retires only strictly earlier epochs.
RetireRoot ==
    /\ \E root \in roots, receipt \in receipts :
        /\ (receipt.owner = root.owner \/ RetireIgnoresOwner)
        /\ (root.epoch < receipt.fence \/ RetireIgnoresEpoch)
        /\ roots' = roots \ {root}
        /\ retired' = retired \cup {root}
    /\ UNCHANGED <<phase, owner, epoch, nextEpoch, leases, nextLease,
                   receipts, vanished>>

Next ==
    \/ StartWorker
    \/ \E c \in IF TwoClients THEN Clients ELSE {1} :
        Acquire(c) \/ Release(c) \/ Quarantine(c)
    \/ DrainStale \/ RequestStop \/ StopDone \/ ReconcileStop
    \/ Crash \/ Reprovision \/ Reincarnate \/ Restart \/ PublishRoot \/ RetireRoot

Init == /\ phase = "provisioning"
        /\ owner = 1
        /\ epoch = 0
        /\ nextEpoch = 1
        /\ leases = {}
        /\ nextLease = 1
        /\ roots = {}
        /\ receipts = {}
        /\ retired = {}
        /\ vanished = FALSE

Spec == Init /\ [][Next]_variables

TypeOK ==
    /\ phase \in States
    /\ owner \in 1..MaxOwner
    /\ epoch \in 0..MaxEpoch
    /\ nextEpoch \in 1..(MaxEpoch + 1)
    /\ nextLease \in 1..(MaxLeaseId + 1)
    /\ leases \subseteq LeaseSpace
    /\ roots \subseteq RootSpace
    /\ receipts \subseteq ReceiptSpace
    /\ retired \subseteq RootSpace
    /\ vanished \in BOOLEAN

NoStopWithLiveLease ==
    phase \in {"stopping", "stopped"} => LeaseFree

NoLeaseUnlessLeasable ==
    phase \in {"provisioning", "stopped"} => LeaseFree

NoSilentLeaseVanish == ~vanished

\* A live or stale lease always names the CURRENT owner incarnation and
\* epoch; reincarnation and new starts are impossible while it exists.
LeaseBindsCurrentIncarnation ==
    \A l \in leases : l.owner = owner /\ l.epoch = epoch

\* Lease identities are unique and always below the durable counter, which
\* never resets within or across incarnations.
UniqueLeaseIds == \A a, b \in leases : a.id = b.id => a = b
LeaseCounterAhead == \A l \in leases : l.id < nextLease

\* Durable epoch fence: the current epoch is strictly below the durable
\* next-epoch counter of the same incarnation.
EpochBelowFence == epoch < nextEpoch
ReadyHasEpoch == phase = "ready" => epoch \in 1..MaxEpoch

\* Every retired root was retired under a receipt of its own owner
\* incarnation and strictly after its epoch.
RetireRespectsOwner ==
    \A r \in retired : \E receipt \in receipts : receipt.owner = r.owner
RetireRespectsEpoch ==
    \A r \in retired : \E receipt \in receipts : r.epoch < receipt.fence

InvariantConjunction ==
    /\ TypeOK
    /\ NoStopWithLiveLease
    /\ NoLeaseUnlessLeasable
    /\ NoSilentLeaseVanish
    /\ LeaseBindsCurrentIncarnation
    /\ UniqueLeaseIds
    /\ LeaseCounterAhead
    /\ EpochBelowFence
    /\ ReadyHasEpoch
    /\ RetireRespectsOwner
    /\ RetireRespectsEpoch
=============================================================================
