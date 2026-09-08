----------------------------- MODULE MergeBoundary -----------------------------
(***************************************************************************
0044 single-module merge contract. Closed-block modes, edit flags and malformed
marker evidence are parser INPUTS, not a proof of the parser. managed_blocks
tests/fuzz and CLI flows bridge these decisions to real token streams.
The model preserves existing managed-block content self-healing, but malformed
boundaries or conflicting mode evidence cannot authorize mutation. The foreign
text variable abstracts every byte outside complete owned spans, including an
unclosed marker's following tail. FileMode/Transaction cover other dimensions.
***************************************************************************)
EXTENDS Naturals, Sequences
CONSTANTS IgnoreUnclosedMarker, InspectFirstModeOnly, PruneFirstBlockOnly
Modes == {"private", "public", "unknown"}
Block == [mode: Modes, edited: BOOLEAN]
BlockSequences == UNION {[1..count -> Block] : count \in 0..2}
VARIABLES blocks, liveMode, malformed, receiptIntact, contentChanges, operation,
          phase, mutated, foreignText, priorForeignText
variables == <<blocks, liveMode, malformed, receiptIntact, contentChanges, operation,
               phase, mutated, foreignText, priorForeignText>>

ModeConflictAt(index) == blocks[index].mode # "unknown" /\ blocks[index].mode # liveMode
AnyModeConflict == \E index \in 1..Len(blocks) : ModeConflictAt(index)
ObservedModeConflict == IF InspectFirstModeOnly
    THEN IF Len(blocks) = 0 THEN FALSE ELSE ModeConflictAt(1)
    ELSE AnyModeConflict
AllContentIntact == \A index \in 1..Len(blocks) : ~blocks[index].edited

Init ==
    /\ blocks \in BlockSequences /\ liveMode \in Modes \ {"unknown"}
    /\ malformed \in BOOLEAN /\ receiptIntact \in BOOLEAN
    /\ contentChanges \in BOOLEAN /\ operation \in {"apply", "prune"}
    /\ phase = "inspect" /\ mutated = FALSE
    /\ foreignText = "outside-complete-owned-spans"
    /\ priorForeignText = foreignText

Reconcile ==
    /\ phase = "inspect"
    /\ LET shapeSafe == ~malformed \/ IgnoreUnclosedMarker
           modeSafe == ~ObservedModeConflict
           pruneSafe == receiptIntact /\ (IF PruneFirstBlockOnly
               THEN Len(blocks) > 0 /\ ~blocks[1].edited
               ELSE Len(blocks) = 1 /\ AllContentIntact)
           needed == contentChanges \/ Len(blocks) # 1 \/ ~AllContentIntact
       IN mutated' = (shapeSafe /\ modeSafe /\ (IF operation = "prune" THEN pruneSafe ELSE needed))
    /\ foreignText' = IF mutated' /\ malformed THEN "tail-lost" ELSE foreignText
    /\ phase' = "done"
    /\ UNCHANGED <<blocks, liveMode, malformed, receiptIntact, contentChanges,
                   operation, priorForeignText>>

Next == Reconcile
Spec == Init /\ [][Next]_variables /\ WF_variables(Reconcile)
TypeOK == /\ blocks \in BlockSequences /\ phase \in {"inspect", "done"}
          /\ mutated \in BOOLEAN
ForeignTextPreserved == foreignText = priorForeignText
MalformedNeverMutates == malformed => ~mutated
AllModeEvidenceRequired == AnyModeConflict => ~mutated
PruneNeedsWholeEvidence == operation = "prune" /\ mutated
    => receiptIntact /\ Len(blocks) = 1 /\ AllContentIntact
EventuallyInspected == <>(phase = "done")
=============================================================================
