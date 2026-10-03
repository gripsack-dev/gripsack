-------------------- MODULE WorkerLeaseFenceWitness --------------------
(***************************************************************************
Negative calibration only. No induction theorem is imported: each witness
proves a concrete safe pre-state, then proves the mutant-guarded transition
exists and breaks exactly the named invariant. TLC supplies reachable
instances of the same unsafe switches via the worker-lease mutant configs.
***************************************************************************)
EXTENDS WorkerLease, TLAPS

EmptyLeases == {}
OneLease == {[id |-> 1, client |-> 1, owner |-> 1, epoch |-> 1]}

\* A ready worker under owner incarnation 1/epoch 1 with one live lease.
LiveReady ==
    /\ phase = "ready"
    /\ owner = 1
    /\ epoch = 1
    /\ nextEpoch = 2
    /\ leases = OneLease
    /\ nextLease = 2
    /\ roots = {}
    /\ receipts = {}
    /\ retired = {}
    /\ vanished = FALSE

THEOREM LiveReadyStartsSafe == LiveReady => InvariantConjunction
<1>1. LiveReady => TypeOK
    BY SMT DEF LiveReady, OneLease, EmptyLeases, TypeOK, LeaseSpace, RootSpace, ReceiptSpace, States, Clients, MaxEpoch, MaxLeaseId, MaxOwner
<1>2. LiveReady => (NoStopWithLiveLease /\ NoLeaseUnlessLeasable /\ NoSilentLeaseVanish /\ EpochBelowFence /\ ReadyHasEpoch)
    BY SMT DEF LiveReady, OneLease, EmptyLeases, NoStopWithLiveLease, NoLeaseUnlessLeasable, NoSilentLeaseVanish, EpochBelowFence, ReadyHasEpoch, LeaseFree, MaxEpoch
<1>3. LiveReady => (LeaseBindsCurrentIncarnation /\ UniqueLeaseIds /\ LeaseCounterAhead)
    BY SMT DEF LiveReady, OneLease, EmptyLeases, LeaseBindsCurrentIncarnation, UniqueLeaseIds, LeaseCounterAhead
<1>4. LiveReady => (RetireRespectsOwner /\ RetireRespectsEpoch)
    BY SMT DEF LiveReady, OneLease, EmptyLeases, RetireRespectsOwner, RetireRespectsEpoch
<1>5. QED
    BY SMT, <1>1, <1>2, <1>3, <1>4 DEF InvariantConjunction

StoppedWithLiveLease ==
    /\ LiveReady
    /\ phase' = "stopping"
    /\ owner' = 1 /\ epoch' = 1 /\ nextEpoch' = 2
    /\ leases' = OneLease /\ nextLease' = 2
    /\ roots' = {} /\ receipts' = {} /\ retired' = {} /\ vanished' = FALSE

THEOREM MissingStopFenceStopsWithLiveLease ==
    ASSUME StopIgnoresLeases, StoppedWithLiveLease
    PROVE RequestStop /\ ~NoStopWithLiveLease'
    BY SMT
    DEF StoppedWithLiveLease, LiveReady, OneLease, EmptyLeases, RequestStop,
        NoStopWithLiveLease, LeaseFree, MaxEpoch, MaxLeaseId, MaxOwner

CrashVanishesLease ==
    /\ LiveReady
    /\ phase' = "failed"
    /\ owner' = 1 /\ epoch' = 1 /\ nextEpoch' = 2
    /\ leases' = {} /\ nextLease' = 2
    /\ roots' = {} /\ receipts' = {} /\ retired' = {} /\ vanished' = TRUE

THEOREM MissingLeaseFenceErasesCrashEvidence ==
    ASSUME CrashEnabled, CrashWipesLeases, CrashVanishesLease
    PROVE Crash /\ ~NoSilentLeaseVanish'
    BY SMT
    DEF CrashVanishesLease, LiveReady, OneLease, EmptyLeases, Crash,
        NoSilentLeaseVanish, LeaseFree

\* A drained stopped worker whose last stop receipt fences only epoch < 2,
\* with a retained root published at epoch 2 by a LATER start.
StaleEpochState ==
    /\ phase = "stopped"
    /\ owner = 1
    /\ epoch = 2
    /\ nextEpoch = 3
    /\ leases = EmptyLeases
    /\ nextLease = 1
    /\ roots = {[owner |-> 1, epoch |-> 2]}
    /\ receipts = {[owner |-> 1, fence |-> 2]}
    /\ retired = {}
    /\ vanished = FALSE

THEOREM StaleEpochStateStartsSafe == StaleEpochState => InvariantConjunction
<1>1. StaleEpochState => TypeOK
    BY SMT DEF StaleEpochState, EmptyLeases, TypeOK, LeaseSpace, RootSpace, ReceiptSpace, States, Clients, MaxEpoch, MaxLeaseId, MaxOwner
<1>2. StaleEpochState => (NoStopWithLiveLease /\ NoLeaseUnlessLeasable /\ NoSilentLeaseVanish /\ EpochBelowFence /\ ReadyHasEpoch)
    BY SMT DEF StaleEpochState, EmptyLeases, NoStopWithLiveLease, NoLeaseUnlessLeasable, NoSilentLeaseVanish, EpochBelowFence, ReadyHasEpoch, LeaseFree, MaxEpoch
<1>3. StaleEpochState => (LeaseBindsCurrentIncarnation /\ UniqueLeaseIds /\ LeaseCounterAhead)
    BY SMT DEF StaleEpochState, EmptyLeases, LeaseBindsCurrentIncarnation, UniqueLeaseIds, LeaseCounterAhead
<1>4. StaleEpochState => (RetireRespectsOwner /\ RetireRespectsEpoch)
    BY SMT DEF StaleEpochState, EmptyLeases, RetireRespectsOwner, RetireRespectsEpoch
<1>5. QED
    BY SMT, <1>1, <1>2, <1>3, <1>4 DEF InvariantConjunction

StaleEpochRetirement ==
    /\ StaleEpochState
    /\ phase' = "stopped"
    /\ owner' = 1 /\ epoch' = 2 /\ nextEpoch' = 3
    /\ leases' = EmptyLeases /\ nextLease' = 1
    /\ roots' = {}
    /\ receipts' = {[owner |-> 1, fence |-> 2]}
    /\ retired' = {[owner |-> 1, epoch |-> 2]}
    /\ vanished' = FALSE

THEOREM MissingEpochFenceRetiresNewEpochRoot ==
    ASSUME RetireIgnoresEpoch, StaleEpochRetirement
    PROVE RetireRoot /\ ~RetireRespectsEpoch'
    BY SMT
    DEF StaleEpochRetirement, StaleEpochState, EmptyLeases, RetireRoot,
        RetireRespectsEpoch, LeaseFree

\* A reincarnated namespace (owner 2) whose new build root is matched by an
\* OLD owner-1 stop receipt when the owner fence is ignored.
ForeignOwnerState ==
    /\ phase = "stopped"
    /\ owner = 2
    /\ epoch = 1
    /\ nextEpoch = 2
    /\ leases = EmptyLeases
    /\ nextLease = 1
    /\ roots = {[owner |-> 2, epoch |-> 1]}
    /\ receipts = {[owner |-> 1, fence |-> 2]}
    /\ retired = {}
    /\ vanished = FALSE

THEOREM ForeignOwnerStateStartsSafe == ForeignOwnerState => InvariantConjunction
<1>1. ForeignOwnerState => TypeOK
    BY SMT DEF ForeignOwnerState, EmptyLeases, TypeOK, LeaseSpace, RootSpace, ReceiptSpace, States, Clients, MaxEpoch, MaxLeaseId, MaxOwner
<1>2. ForeignOwnerState => (NoStopWithLiveLease /\ NoLeaseUnlessLeasable /\ NoSilentLeaseVanish /\ EpochBelowFence /\ ReadyHasEpoch)
    BY SMT DEF ForeignOwnerState, EmptyLeases, NoStopWithLiveLease, NoLeaseUnlessLeasable, NoSilentLeaseVanish, EpochBelowFence, ReadyHasEpoch, LeaseFree, MaxEpoch
<1>3. ForeignOwnerState => (LeaseBindsCurrentIncarnation /\ UniqueLeaseIds /\ LeaseCounterAhead)
    BY SMT DEF ForeignOwnerState, EmptyLeases, LeaseBindsCurrentIncarnation, UniqueLeaseIds, LeaseCounterAhead
<1>4. ForeignOwnerState => (RetireRespectsOwner /\ RetireRespectsEpoch)
    BY SMT DEF ForeignOwnerState, EmptyLeases, RetireRespectsOwner, RetireRespectsEpoch
<1>5. QED
    BY SMT, <1>1, <1>2, <1>3, <1>4 DEF InvariantConjunction

ForeignOwnerRetirement ==
    /\ ForeignOwnerState
    /\ phase' = "stopped"
    /\ owner' = 2 /\ epoch' = 1 /\ nextEpoch' = 2
    /\ leases' = EmptyLeases /\ nextLease' = 1
    /\ roots' = {}
    /\ receipts' = {[owner |-> 1, fence |-> 2]}
    /\ retired' = {[owner |-> 2, epoch |-> 1]}
    /\ vanished' = FALSE

THEOREM MissingOwnerFenceRetiresNewOwnerRoot ==
    ASSUME RetireIgnoresOwner, ForeignOwnerRetirement
    PROVE RetireRoot /\ ~RetireRespectsOwner'
    BY SMT
    DEF ForeignOwnerRetirement, ForeignOwnerState, EmptyLeases, RetireRoot,
        RetireRespectsOwner, LeaseFree
=============================================================================
