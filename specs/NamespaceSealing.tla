---- MODULE NamespaceSealing ----
EXTENDS Integers

CONSTANTS Depth, Mutant
ASSUME Parameters ==
    /\ Depth \in Nat
    /\ Mutant \in {"none", "skip_existing", "leaf_only"}

Names == 1..Depth
Phases == {"sealing", "ready", "done", "stopped"}
VARIABLES nodes, names, cachedReachable, cursor, phase, returned
vars == <<nodes, names, cachedReachable, cursor, phase, returned>>

\* Root 0 is the supplied filesystem/capability anchor. A successful lookup
\* establishes only that the complete path is cached-visible. Every created
\* inode and parent-name edge may still be missing from stable storage.
Init ==
    /\ nodes \in SUBSET Names /\ names \in SUBSET Names
    /\ cachedReachable = TRUE /\ cursor = Depth /\ returned = FALSE
    /\ phase = IF Depth = 0 \/ Mutant = "skip_existing" THEN "ready" ELSE "sealing"

\* TLC requires explicit successor assignments when a parent starts a fresh
\* namespace episode. ResetMatchesInit proves this constructor equals Init'.
Reset ==
    /\ nodes' \in SUBSET Names /\ names' \in SUBSET Names
    /\ cachedReachable' = TRUE /\ cursor' = Depth /\ returned' = FALSE
    /\ phase' = IF Depth = 0 \/ Mutant = "skip_existing" THEN "ready" ELSE "sealing"

AllSealed == nodes = Names /\ names = Names

\* Sync of directory i seals its inode and its immediate child's name.
\* No action is allowed to assume that flushing an ancestor flushes a child.
SyncDirectory ==
    /\ phase = "sealing" /\ cachedReachable
    /\ nodes' = IF cursor = 0 THEN nodes ELSE nodes \union {cursor}
    /\ names' = IF cursor < Depth THEN names \union {cursor + 1} ELSE names
    /\ phase' = IF cursor = 0 \/ Mutant = "leaf_only" THEN "ready" ELSE "sealing"
    /\ cursor' = IF cursor = 0 THEN 0 ELSE cursor - 1
    /\ UNCHANGED <<cachedReachable, returned>>
ReturnAuthority ==
    /\ phase = "ready" /\ phase' = "done" /\ returned' = TRUE
    /\ UNCHANGED <<nodes, names, cachedReachable, cursor>>
ProcessDeath ==
    /\ phase' = "stopped" /\ cursor' = Depth
    /\ UNCHANGED <<nodes, names, cachedReachable, returned>>
ResumeObserved ==
    /\ phase = "stopped" /\ cachedReachable
    /\ cursor' = Depth
    /\ phase' = IF Depth = 0 \/ Mutant = "skip_existing" THEN "ready" ELSE "sealing"
    /\ UNCHANGED <<nodes, names, cachedReachable, returned>>
Writeback ==
    /\ cachedReachable
    /\ nodes' \in {next \in SUBSET Names : nodes \subseteq next}
    /\ names' \in {next \in SUBSET Names : names \subseteq next}
    /\ UNCHANGED <<cachedReachable, cursor, phase, returned>>
PowerLoss ==
    /\ IF cachedReachable
       THEN /\ nodes' \in {next \in SUBSET Names : nodes \subseteq next}
            /\ names' \in {next \in SUBSET Names : names \subseteq next}
       ELSE UNCHANGED <<nodes, names>>
    /\ cachedReachable' = (nodes' = Names /\ names' = Names)
    /\ phase' = "stopped" /\ cursor' = Depth
    /\ UNCHANGED returned

Next == SyncDirectory \/ ReturnAuthority \/ ProcessDeath \/ ResumeObserved \/ Writeback \/ PowerLoss
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ nodes \in SUBSET Names /\ names \in SUBSET Names
    /\ cachedReachable \in BOOLEAN /\ returned \in BOOLEAN
    /\ cursor \in 0..Depth /\ phase \in Phases
ControlInvariant ==
    /\ (phase = "sealing" =>
          /\ cachedReachable
          /\ (cursor + 1)..Depth \subseteq nodes
          /\ (cursor + 2)..Depth \subseteq names)
    /\ (phase \in {"ready", "done"} => cachedReachable /\ AllSealed)
AuthorityHasDurableNamespace == returned => cachedReachable /\ AllSealed
Invariant == TypeOK /\ ControlInvariant /\ AuthorityHasDurableNamespace

=============================================================================
