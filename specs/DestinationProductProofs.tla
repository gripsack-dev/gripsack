---- MODULE DestinationProductProofs ----
EXTENDS DestinationProduct, UndoCellProofs, FunctionFrames

THEOREM ProductInitialSafety == Init => Invariant
  BY SMT, InitialCellSetSafety
  DEF Init, Invariant, CellInvariant

THEOREM DestinationTransitionSafety ==
  ASSUME Invariant, NEW destination \in Destinations, DestinationStep(destination)
  PROVE Invariant'
<1>1. PICK next \in CellSuccessors(cells[destination], committed) :
        cells' = [cells EXCEPT ![destination] = next]
  BY DEF DestinationStep
<1>2. CellInvariant(next, committed)
  BY SMT, CellStepSafety, <1>1 DEF Invariant, CellSuccessors
<1>3. next \in CellSpace
  BY <1>2 DEF CellInvariant
<1>4. cells' \in [Destinations -> CellSpace]
  BY SMT, ReplacementDomain, <1>1, <1>3 DEF Invariant, ReplaceCell
<1>5. \A other \in Destinations : CellInvariant(cells'[other], committed)
  BY SMT, ReplacementFrame, <1>1, <1>2 DEF Invariant, ReplaceCell
<1>6. QED
  BY <1>4, <1>5 DEF Invariant, DestinationStep

THEOREM IndependentDestinationImagesCommute ==
  ASSUME Invariant,
         NEW first \in Destinations, NEW second \in Destinations, first # second,
         NEW firstCell \in CellSuccessors(cells[first], committed),
         NEW secondCell \in CellSuccessors(cells[second], committed)
  PROVE /\ [cells EXCEPT ![first] = firstCell, ![second] = secondCell]
            = [cells EXCEPT ![second] = secondCell, ![first] = firstCell]
        /\ secondCell \in CellSuccessors([cells EXCEPT ![first] = firstCell][second], committed)
        /\ firstCell \in CellSuccessors([cells EXCEPT ![second] = secondCell][first], committed)
<1>1. firstCell \in CellSpace /\ secondCell \in CellSpace
  BY SMT, CellStepSafety DEF Invariant, CellSuccessors, CellInvariant
<1>2. QED
  BY SMT, IndependentReplacementsCommute, ReplacementFrame, <1>1
  DEF Invariant, ReplaceCell

THEOREM IndependentPowerLossSafety == Invariant /\ PowerLossAll => Invariant'
<1> SUFFICES ASSUME Invariant, PowerLossAll PROVE Invariant'
  OBVIOUS
<1>1. \A destination \in Destinations : CellInvariant(cells'[destination], committed)
  <2> TAKE destination \in Destinations
  <2>1. PowerLoss(cells[destination], cells'[destination])
    BY SMT DEF PowerLossAll, PowerLossChoices, PowerLossImages, PowerLoss
  <2>2. QED
    BY SMT, PowerLossSafety, <2>1 DEF Invariant
<1>2. cells' \in [Destinations -> CellSpace]
  BY SMT, <1>1 DEF CellInvariant, PowerLossAll, PowerLossChoices
<1>3. QED
  BY <1>1, <1>2 DEF Invariant, PowerLossAll

THEOREM MonotoneCommitInterfaceSafety == Invariant /\ CommitAuthority => Invariant'
  BY SMT, CommitMonotonicity DEF Invariant, CommitAuthority

THEOREM ProductInduction == Invariant /\ Next => Invariant'
  BY SMT, DestinationTransitionSafety, IndependentPowerLossSafety,
     MonotoneCommitInterfaceSafety
  DEF Next

THEOREM ProductInvariantImpliesRecoverySafety == Invariant => RecoveryEvidencePreserved
  BY SMT, NoLostUndoEvidence DEF Invariant, RecoveryEvidencePreserved

THEOREM ArbitraryDestinationSafety == Spec => []Invariant
<1>1. Init => Invariant
  BY ProductInitialSafety
<1>2. Invariant /\ [Next]_vars => Invariant'
  BY ProductInduction DEF Invariant, vars
<1>3. QED
  BY PTL, <1>1, <1>2 DEF Spec

THEOREM ArbitraryDestinationRecoveryEvidence == Spec => []RecoveryEvidencePreserved
  BY PTL, ArbitraryDestinationSafety, ProductInvariantImpliesRecoverySafety

=============================================================================
