--------------------------- MODULE WorkerLease ---------------------------
(***************************************************************************
B1 worker lease safety (Epic B 10.3 / plan 0051 B1 fences): a managed
worker may not stop while a live lease requires it, leases never leak
into a stopped worker, a crash never silently erases leases, and
recovery is blocked until stale leases are drained. Two clients
interleave; the environment may crash the worker in any live moment.
Mirrors the production transition table in gripsack_buildkit::worker.
The mutant switches (StopIgnoresLeases, CrashWipesLeases) exist ONLY
in the calibrated negative configs and are FALSE in the positive one;
provisioning effects, sockets, VMs and timing are NOT modeled.
***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS TwoClients, CrashEnabled, StopIgnoresLeases, CrashWipesLeases
Clients == {1, 2}
States == {"provisioning", "ready", "stopping", "stopped", "failed"}
MaxLeaseId == 3

VARIABLES phase, leases, nextId, vanished
variables == <<phase, leases, nextId, vanished>>

LeaseCount(les) == Cardinality(DOMAIN les)

ProvisionDone ==
    /\ phase = "provisioning"
    /\ phase' = "ready"
    /\ UNCHANGED <<leases, nextId, vanished>>

Acquire(c) ==
    /\ phase = "ready"
    /\ nextId <= MaxLeaseId
    /\ leases' = [l \in (DOMAIN leases \cup {nextId}) |-> IF l = nextId THEN c ELSE leases[l]]
    /\ nextId' = nextId + 1
    /\ UNCHANGED <<phase, vanished>>

Release(c) ==
    /\ phase \in {"ready", "stopping"}
    /\ \E l \in DOMAIN leases : leases[l] = c
    /\ leases' = [l \in {x \in DOMAIN leases : leases[x] # c} |-> leases[l]]
    /\ UNCHANGED <<phase, nextId, vanished>>

RequestStop ==
    /\ phase = "ready"
    /\ LeaseCount(leases) = 0 \/ StopIgnoresLeases
    /\ phase' = "stopping"
    /\ UNCHANGED <<leases, nextId, vanished>>

StopDone ==
    /\ phase = "stopping"
    /\ LeaseCount(leases) = 0
    /\ phase' = "stopped"
    /\ UNCHANGED <<leases, nextId, vanished>>

Crash ==
    /\ CrashEnabled
    /\ phase \in {"provisioning", "ready", "stopping"}
    /\ phase' = "failed"
    /\ IF CrashWipesLeases /\ LeaseCount(leases) > 0
       THEN /\ leases' = [l \in {} |-> 0] /\ vanished' = TRUE
       ELSE /\ leases' = leases /\ vanished' = vanished
    /\ UNCHANGED nextId

Recover ==
    /\ phase = "failed"
    /\ LeaseCount(leases) = 0
    /\ phase' = "provisioning"
    /\ nextId' = 1
    /\ UNCHANGED <<leases, vanished>>

Next ==
    \/ ProvisionDone
    \/ \E c \in IF TwoClients THEN Clients ELSE {1} : Acquire(c) \/ Release(c)
    \/ RequestStop \/ StopDone \/ Recover \/ Crash

Init == /\ phase = "provisioning"
        /\ leases = [l \in {} |-> 0]
        /\ nextId = 1
        /\ vanished = FALSE

Spec == Init /\ [][Next]_variables

TypeOK ==
    /\ phase \in States
    /\ \A l \in DOMAIN leases : leases[l] \in Clients
    /\ vanished \in BOOLEAN

NoStopWithLiveLease ==
    phase \in {"stopping", "stopped"} => LeaseCount(leases) = 0

NoLeaseUnlessLeasable ==
    phase \in {"provisioning", "stopped"} => LeaseCount(leases) = 0

NoSilentLeaseVanish == ~vanished

InvariantConjunction ==
    /\ TypeOK
    /\ NoStopWithLiveLease
    /\ NoLeaseUnlessLeasable
    /\ NoSilentLeaseVanish
=============================================================================
