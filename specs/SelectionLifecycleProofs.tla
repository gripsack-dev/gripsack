---- MODULE SelectionLifecycleProofs ----
EXTENDS SelectionLifecycle, TLAPS

ASSUME CorrectProtocol == Mutant = "none"

THEOREM SelectionInitialSafety == Init => Invariant
  BY SMT, Parameters
  DEF Init, Invariant, TypeOK, ReservationInvariant, CurrentInvariant, MarkerInvariant,
      Bound, IsTransaction, InitialTransactions, EmptyMarker, MarkerSpace, Selections

THEOREM SelectionInduction ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW clean \in BOOLEAN, NEW admitted \in BOOLEAN,
         Invariant, Next(available, clean, admitted)
  PROVE Invariant'
<1> USE DEF Invariant, TypeOK, ReservationInvariant, CurrentInvariant, MarkerInvariant,
           Bound, IsTransaction, MarkerSpace, Selections
<1>1. CASE \E transaction \in Transactions : \E generation \in GenerationIds : Reserve(transaction, generation)
  BY SMT, Parameters, <1>1 DEF Reserve
<1>2. CASE PublishLink
  BY SMT, Parameters, <1>2 DEF PublishLink
<1>3. CASE SealLink
  BY SMT, Parameters, <1>3 DEF SealLink
<1>4. CASE SealReservations
  BY SMT, Parameters, <1>4 DEF SealReservations
<1>5. CASE WriteMarker
  BY SMT, Parameters, <1>5 DEF WriteMarker, Marker
<1>6. CASE SealMarker
  BY SMT, Parameters, <1>6 DEF SealMarker
<1>7. CASE Flip(available)
  BY SMT, Parameters, <1>7 DEF Flip
<1>8. CASE SealCurrent
  BY SMT, Parameters, <1>8 DEF SealCurrent
<1>9. CASE ClearMarker(clean)
  BY SMT, Parameters, <1>9 DEF ClearMarker, EmptyMarker
<1>10. CASE ProcessDeath
  BY SMT, Parameters, <1>10 DEF ProcessDeath
<1>11. CASE PowerLoss
  BY SMT, Parameters, <1>11 DEF PowerLoss
<1>12. CASE \E selection \in Selections : ForeignCurrent(available, selection)
  BY SMT, Parameters, <1>12 DEF ForeignCurrent
<1>13. QED
  BY SMT, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12
  DEF Next, CoreNext, StorageNext

THEOREM NewlyReservedIdentityIsFresh ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds,
         Invariant, Reserve(transaction, generation)
  PROVE transaction \notin reservationStable /\ transaction \notin issued
  BY SMT DEF Invariant, ReservationInvariant, Reserve

THEOREM MarkerCannotCommitItsPredecessor == Invariant => PredecessorCannotCommit
  BY SMT, CorrectProtocol DEF Invariant, MarkerInvariant, PredecessorCannotCommit, Classify

THEOREM FlipNamesTheExactTarget ==
  ASSUME NEW available, Flip(available)
  PROVE currentVisible' = markerStable.target /\ currentVisible'[2] \in available
  BY SMT DEF Flip

THEOREM AmbiguityCannotClearEvidence ==
  ASSUME NEW clean, markerVisible.present,
         Classify(markerVisible.previous, markerVisible.target, currentVisible) = "ambiguous"
  PROVE ~ClearMarker(clean)
  BY DEF ClearMarker

THEOREM DurableReservationPersists ==
  ASSUME NEW available, NEW clean, NEW admitted, Invariant, Next(available, clean, admitted)
  PROVE reservationStable \subseteq reservationStable'
  BY SMT
  DEF Invariant, TypeOK, ReservationInvariant, Next, CoreNext, StorageNext,
      Reserve, PublishLink, SealLink, SealReservations,
      WriteMarker, SealMarker, Flip, SealCurrent, ClearMarker, ProcessDeath, PowerLoss, ForeignCurrent

THEOREM IssuedBindingNeverChanges ==
  ASSUME NEW available, NEW clean, NEW admitted, Invariant, Next(available, clean, admitted)
  PROVE \A transaction \in issued : binding'[transaction] = binding[transaction]
  BY SMT, Parameters
  DEF Invariant, TypeOK, ReservationInvariant, Next, CoreNext, StorageNext, Reserve,
      PublishLink, SealLink, SealReservations, WriteMarker, SealMarker, Flip,
      SealCurrent, ClearMarker, ProcessDeath, PowerLoss, ForeignCurrent

THEOREM SelectionSafety == Spec => []Invariant
<1>1. Init => Invariant
  BY SelectionInitialSafety
<1>2. Invariant /\ [Next(AvailableGenerations, TRUE, TRUE)]_vars => Invariant'
  BY SMT, Parameters, SelectionInduction
  DEF Invariant, TypeOK, ReservationInvariant, CurrentInvariant, MarkerInvariant, Bound, vars
<1>3. QED
  BY PTL, <1>1, <1>2 DEF Spec

THEOREM ExactTransactionPredecessorSafety == Spec => []PredecessorCannotCommit
  BY PTL, SelectionSafety, MarkerCannotCommitItsPredecessor

=============================================================================
