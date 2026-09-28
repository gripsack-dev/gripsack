---- MODULE UndoCellCheck ----
EXTENDS UndoCell
VARIABLES cell, committed
vars == <<cell, committed>>
Init == /\ cell \in InitialCells
        /\ committed = FALSE
Next == \/ /\ CellStep(cell, cell', committed)
           /\ UNCHANGED committed
        \/ /\ ~committed /\ committed' = TRUE /\ UNCHANGED cell
Spec == Init /\ [][Next]_vars
Invariant == CellInvariant(cell, committed)
=============================================================================
