---- MODULE DestinationProduct ----
EXTENDS UndoCell

CONSTANT Destinations
ASSUME DestinationDomain == IsFiniteSet(Destinations)

VARIABLES cells, committed
vars == <<cells, committed>>

Init ==
    /\ cells \in [Destinations -> InitialCells]
    /\ committed = FALSE

DestinationStep(destination) ==
    /\ \E next \in CellSuccessors(cells[destination], committed) :
          cells' = [cells EXCEPT ![destination] = next]
    /\ UNCHANGED committed

\* Independent old/new choices for every destination. An empty destination
\* set has the unique empty function, not a fabricated non-empty witness.
PowerLossChoices ==
    {next \in [Destinations -> UNION {PowerLossImages(cells[d]) : d \in Destinations}] :
        \A d \in Destinations : next[d] \in PowerLossImages(cells[d])}
PowerLossAll == /\ cells' \in PowerLossChoices /\ UNCHANGED committed

\* This is the local product interface, not a commit-storage operation.
\* The lifecycle composition must establish when exact durable selection
\* authority can change FALSE to TRUE. It may never change back within a run.
CommitAuthority == /\ ~committed /\ committed' = TRUE /\ UNCHANGED cells

Next == (\E destination \in Destinations : DestinationStep(destination))
        \/ PowerLossAll \/ CommitAuthority
Spec == Init /\ [][Next]_vars

Invariant ==
    /\ cells \in [Destinations -> CellSpace]
    /\ committed \in BOOLEAN
    /\ \A destination \in Destinations : CellInvariant(cells[destination], committed)

RecoveryEvidencePreserved ==
    ~committed =>
      \A destination \in Destinations :
        ~cells[destination].durable.entry.present =>
          ~UnsafeOwned(cells[destination], cells[destination].durable.live)

=============================================================================
