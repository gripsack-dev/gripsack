---- MODULE DestinationWrites ----
(***************************************************************************)
(* Journal v2's repeated-destination seam. Two durable records/writes,       *)
(* crashes before/after either write, and post-crash foreign edits.           *)
(* Record corresponds to journal::record/Entry::advance; Recover to the      *)
(* production decide_from(before, intended, original) adapter. The Rust      *)
(* filesystem cases exercise both crash windows with actual blob/record IO. *)
(* This finite TLC check is not the generalized M-V6/TLAPS theorem.          *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets
CONSTANTS ORIGINAL, FIRST, SECOND, FOREIGN, NONE, BadPrior, BadBefore
ASSUME Cardinality({ORIGINAL, FIRST, SECOND, FOREIGN, NONE}) = 5
Values == {ORIGINAL, FIRST, SECOND, FOREIGN}
VARIABLES dest, entry, phase, writes, edited
vars == <<dest, entry, phase, writes, edited>>

Init ==
    /\ dest = ORIGINAL
    /\ entry = NONE
    /\ phase = "running"
    /\ writes = 0
    /\ edited = FALSE

Record ==
    /\ phase = "running"
    /\ writes < 2
    /\ IF entry = NONE THEN TRUE ELSE dest = entry.after
    /\ \E after \in {FIRST, SECOND} :
        entry' = [prior |-> IF entry = NONE THEN dest
                           ELSE IF BadPrior THEN dest ELSE entry.prior,
                  before |-> IF BadBefore THEN ORIGINAL ELSE dest,
                  after |-> after]
    /\ phase' = "recorded"
    /\ UNCHANGED <<dest, writes, edited>>

Write ==
    /\ phase = "recorded"
    /\ dest' = entry.after
    /\ writes' = writes + 1
    /\ phase' = "running"
    /\ UNCHANGED <<entry, edited>>

Crash ==
    /\ phase \in {"running", "recorded"}
    /\ phase' = "crashed"
    /\ UNCHANGED <<dest, entry, writes, edited>>

ForeignEdit ==
    /\ phase = "crashed"
    /\ dest' = FOREIGN
    /\ edited' = TRUE
    /\ UNCHANGED <<entry, phase, writes>>

Recover ==
    /\ phase = "crashed"
    /\ dest' = IF entry = NONE THEN dest
               ELSE IF dest = entry.after THEN entry.prior
               ELSE IF dest = entry.prior THEN dest
               ELSE IF dest = entry.before THEN entry.prior
               ELSE dest
    /\ entry' = NONE
    /\ phase' = "done"
    /\ UNCHANGED <<writes, edited>>

Next == Record \/ Write \/ Crash \/ ForeignEdit \/ Recover
        \/ (phase = "done" /\ UNCHANGED vars)
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ dest \in Values
    /\ entry \in {NONE} \union [prior: Values, before: Values, after: {FIRST, SECOND}]
    /\ phase \in {"running", "recorded", "crashed", "done"}
    /\ writes \in 0..2
    /\ edited \in BOOLEAN
OriginalPriorPreserved == IF entry = NONE THEN TRUE ELSE entry.prior = ORIGINAL
NoPartialRecovery == phase = "done" => dest = IF edited THEN FOREIGN ELSE ORIGINAL
NeverTwoWrites == writes < 2
=============================================================================
