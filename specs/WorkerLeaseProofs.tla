------------------------ MODULE WorkerLeaseProofs ------------------------
(***************************************************************************
Inductive safety of the positive WorkerLease protocol (all mutant switches
FALSE — the calibrated negative configs and WorkerLeaseFenceWitness cover
the disabled fences). One step theorem per production-mapped action, the
inductive conjunction, and the safety corollaries. TLC bounds and the
Docker/record effects remain separate tested/trusted boundaries.
***************************************************************************)
EXTENDS WorkerLease, TLAPS

ASSUME AdmittedProtocol ==
    /\ TwoClients \in BOOLEAN
    /\ CrashEnabled \in BOOLEAN
    /\ StopIgnoresLeases \in BOOLEAN
    /\ CrashWipesLeases \in BOOLEAN
    /\ RetireIgnoresOwner \in BOOLEAN
    /\ RetireIgnoresEpoch \in BOOLEAN
    /\ ~StopIgnoresLeases /\ ~CrashWipesLeases
    /\ ~RetireIgnoresOwner /\ ~RetireIgnoresEpoch

\* The TLC cfgs pin concrete small bounds; the induction needs only that
\* the bounds are positive naturals.
ASSUME AdmittedBounds ==
    /\ MaxEpoch \in Nat /\ MaxEpoch >= 1
    /\ MaxLeaseId \in Nat /\ MaxLeaseId >= 1
    /\ MaxOwner \in Nat /\ MaxOwner >= 1

THEOREM InitInvariant == Init => InvariantConjunction
    BY SMT, AdmittedBounds
    DEF Init, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM StartInvariant ==
    ASSUME InvariantConjunction, StartWorker
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF StartWorker, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM AcquireInvariant ==
    ASSUME NEW c \in Clients, InvariantConjunction, Acquire(c)
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF Acquire, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM ReleaseInvariant ==
    ASSUME NEW c \in Clients, InvariantConjunction, Release(c)
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF Release, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM QuarantineInvariant ==
    ASSUME NEW c \in Clients, InvariantConjunction, Quarantine(c)
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF Quarantine, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM DrainInvariant ==
    ASSUME InvariantConjunction, DrainStale
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF DrainStale, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM RequestStopInvariant ==
    ASSUME InvariantConjunction, RequestStop
    PROVE InvariantConjunction'
    BY SMT, AdmittedProtocol, AdmittedBounds
    DEF RequestStop, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM StopDoneInvariant ==
    ASSUME InvariantConjunction, StopDone
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF StopDone, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM ReconcileStopInvariant ==
    ASSUME InvariantConjunction, ReconcileStop
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF ReconcileStop, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM CrashInvariant ==
    ASSUME InvariantConjunction, Crash
    PROVE InvariantConjunction'
    BY SMT, AdmittedProtocol, AdmittedBounds
    DEF Crash, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM ReprovisionInvariant ==
    ASSUME InvariantConjunction, Reprovision
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF Reprovision, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM ReincarnateInvariant ==
    ASSUME InvariantConjunction, Reincarnate
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF Reincarnate, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM RestartInvariant ==
    ASSUME InvariantConjunction, Restart
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF Restart, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM PublishInvariant ==
    ASSUME InvariantConjunction, PublishRoot
    PROVE InvariantConjunction'
    BY SMT, AdmittedBounds
    DEF PublishRoot, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM RetireInvariant ==
    ASSUME InvariantConjunction, RetireRoot
    PROVE InvariantConjunction'
    BY SMT, AdmittedProtocol, AdmittedBounds
    DEF RetireRoot, InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, LeaseSpace, RootSpace, ReceiptSpace,
        States, Clients

THEOREM StepInvariant == InvariantConjunction /\ Next => InvariantConjunction'
<1> SUFFICES ASSUME InvariantConjunction, Next
         PROVE InvariantConjunction'
    OBVIOUS
<1>1. CASE StartWorker
    BY SMT, <1>1, StartInvariant
<1>2. CASE \E c \in (IF TwoClients THEN Clients ELSE {1}) : Acquire(c)
    <2>1. PICK c \in (IF TwoClients THEN Clients ELSE {1}) : Acquire(c)
        BY SMT, <1>2
    <2>2. c \in Clients
        BY SMT, AdmittedProtocol, <2>1 DEF Clients
    <2>3. QED
        BY SMT, <2>1, <2>2, AcquireInvariant
<1>3. CASE \E c \in (IF TwoClients THEN Clients ELSE {1}) : Release(c)
    <2>1. PICK c \in (IF TwoClients THEN Clients ELSE {1}) : Release(c)
        BY SMT, <1>3
    <2>2. c \in Clients
        BY SMT, AdmittedProtocol, <2>1 DEF Clients
    <2>3. QED
        BY SMT, <2>1, <2>2, ReleaseInvariant
<1>4. CASE \E c \in (IF TwoClients THEN Clients ELSE {1}) : Quarantine(c)
    <2>1. PICK c \in (IF TwoClients THEN Clients ELSE {1}) : Quarantine(c)
        BY SMT, <1>4
    <2>2. c \in Clients
        BY SMT, AdmittedProtocol, <2>1 DEF Clients
    <2>3. QED
        BY SMT, <2>1, <2>2, QuarantineInvariant
<1>5. CASE DrainStale
    BY SMT, <1>5, DrainInvariant
<1>6. CASE RequestStop
    BY SMT, <1>6, RequestStopInvariant
<1>7. CASE StopDone
    BY SMT, <1>7, StopDoneInvariant
<1>8. CASE ReconcileStop
    BY SMT, <1>8, ReconcileStopInvariant
<1>9. CASE Crash
    BY SMT, <1>9, CrashInvariant
<1>10. CASE Reprovision
    BY SMT, <1>10, ReprovisionInvariant
<1>11. CASE Reincarnate
    BY SMT, <1>11, ReincarnateInvariant
<1>12. CASE Restart
    BY SMT, <1>12, RestartInvariant
<1>13. CASE PublishRoot
    BY SMT, <1>13, PublishInvariant
<1>14. CASE RetireRoot
    BY SMT, <1>14, RetireInvariant
<1>15. QED
    BY SMT, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10,
        <1>11, <1>12, <1>13, <1>14
    DEF Next

THEOREM SafetyCorollaries ==
    InvariantConjunction =>
        /\ NoStopWithLiveLease
        /\ NoLeaseUnlessLeasable
        /\ NoSilentLeaseVanish
        /\ LeaseBindsCurrentIncarnation
        /\ EpochBelowFence
        /\ RetireRespectsOwner
        /\ RetireRespectsEpoch
    BY DEF InvariantConjunction

THEOREM SpecSafety == Spec => []InvariantConjunction
<1>1. Init => InvariantConjunction
    BY InitInvariant
<1>2. InvariantConjunction /\ [Next]_variables => InvariantConjunction'
    BY SMT, StepInvariant
    DEF InvariantConjunction, TypeOK, NoStopWithLiveLease, NoLeaseUnlessLeasable,
        NoSilentLeaseVanish, LeaseBindsCurrentIncarnation, UniqueLeaseIds,
        LeaseCounterAhead, EpochBelowFence, ReadyHasEpoch, RetireRespectsOwner,
        RetireRespectsEpoch, LeaseFree, variables
<1>3. QED
    BY PTL, <1>1, <1>2 DEF Spec
=============================================================================
