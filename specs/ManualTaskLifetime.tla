------------------------ MODULE ManualTaskLifetime ------------------------
EXTENDS Naturals, FiniteSets

(***************************************************************************
Manual invocation subset of the shared lifetime boundary. Each child is an
INSTANCE of the production-corresponding ProcessSupervision model, not a
second cleanup state machine. Two invocations contend for one common mutation
key (the conflicting projection of canonical complete lock sets). An invocation
acquires before its first process step. Claims survive parent crash and failed
cleanup; neither PID reuse, age nor the parent flock supplies release authority.

Production mapping: invocation i = ProcessLease identity, claims = persisted
mutation claim inventory, running = launched command ownership, unknown =
retained indeterminate claims. Collect releases only after confirmed cleanup.
The retention refinement obligation is that unknown ProcessLease payload roots
remain protected like LifecycleRetention!ProtectedRoots before collection.
This model checks retained mutation claims, not payload deletion: its
UnknownRetained invariant does NOT discharge that GC/runner refinement arrow.
It is a manual-invocation projection, not a proof of filesystem claim atomicity,
lease serialization or full graph enumeration. The existing LifecycleRetention
gate remains independently required. Supervisor constants bound byte/time
abstractions; this safety model asserts no liveness.
***************************************************************************)
CONSTANTS IgnoreConflict, DropUnknown
Invocations == {1, 2}
VARIABLES first, second, started, running, collected, claims, unknown, crashed
children == <<first, second>>
vars == <<first, second, started, running, collected, claims, unknown, crashed>>

Child(state) == INSTANCE ProcessSupervision
    WITH s <- state, Deadline <- 2, Reserve <- 1,
         InputLimit <- 1, OutputLimit <- 1, LineLimit <- 1, TailLimit <- 0,
         Mutant <- "none"

Init ==
    /\ Child(first)!Init /\ first.size = 1 /\ first.peer \in {"normal", "fault"}
    /\ Child(second)!Init /\ second.size = 1 /\ second.peer \in {"normal", "fault"}
    /\ started = {} /\ running = {} /\ collected = {}
    /\ claims = {} /\ unknown = {} /\ crashed = {}

Acquire(i) ==
    /\ i \notin started
    /\ claims = {} \/ IgnoreConflict
    /\ started' = started \cup {i}
    /\ running' = running \cup {i}
    /\ claims' = claims \cup {i}
    /\ UNCHANGED <<children, collected, unknown, crashed>>

ProcessStep(i) ==
    /\ i \in running /\ i \notin crashed
    /\ IF i = 1
       THEN /\ Child(first)!Next /\ UNCHANGED second
       ELSE /\ Child(second)!Next /\ UNCHANGED first
    /\ UNCHANGED <<started, running, collected, claims, unknown, crashed>>

Collect(i) ==
    /\ i \in running /\ i \notin crashed
    /\ children[i].phase = "done"
    /\ running' = running \ {i}
    /\ collected' = collected \cup {i}
    /\ IF children[i].cleanup = "failed"
       THEN /\ unknown' = unknown \cup {i}
            /\ claims' = IF DropUnknown THEN claims \ {i} ELSE claims
       ELSE /\ unknown' = unknown
            /\ claims' = claims \ {i}
    /\ UNCHANGED <<children, started, crashed>>

ParentCrash(i) ==
    /\ i \in running
    /\ running' = running \ {i}
    /\ crashed' = crashed \cup {i}
    /\ unknown' = unknown \cup {i}
    /\ claims' = IF DropUnknown THEN claims \ {i} ELSE claims
    /\ UNCHANGED <<children, started, collected>>

Next == \E i \in Invocations :
    Acquire(i) \/ ProcessStep(i) \/ Collect(i) \/ ParentCrash(i)
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ DOMAIN children = Invocations
    /\ \A i \in Invocations : Child(children[i])!TypeOK
    /\ started \subseteq Invocations /\ running \subseteq started
    /\ collected \subseteq started /\ claims \subseteq started
    /\ unknown \subseteq started /\ crashed \subseteq unknown
NoOverlap == Cardinality(running \cup unknown) <= 1
UnknownRetained == unknown \subseteq claims
RunningClaimed == running \subseteq claims
SupervisorSafety == \A i \in Invocations :
    Child(children[i])!BoundedBuffers /\ Child(children[i])!PidAuthority
        /\ Child(children[i])!HonestCleanup
NeverUnknown == unknown = {}
NeverCleanupUnknown == unknown \cap collected = {}
NeverSequential == Cardinality(started) < 2
=============================================================================
