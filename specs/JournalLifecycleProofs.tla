---- MODULE JournalLifecycleProofs ----
EXTENDS JournalLifecycle, UndoCellProofs, FunctionFrames

ASSUME CorrectJournal == JournalMutant = "none" /\ SelectionMutant = "none"
SelectionProofs == INSTANCE SelectionLifecycleProofs WITH Mutant <- SelectionMutant

THEOREM SelectionStateNamesAgree ==
    /\ Selection!Init = SelectionProofs!Init
    /\ Selection!Invariant = SelectionProofs!Invariant
    /\ Selection!vars = selectionVars
  BY DEF Selection!Init, SelectionProofs!Init,
      Selection!Invariant, SelectionProofs!Invariant,
      Selection!TypeOK, SelectionProofs!TypeOK,
      Selection!ReservationInvariant, SelectionProofs!ReservationInvariant,
      Selection!CurrentInvariant, SelectionProofs!CurrentInvariant,
      Selection!MarkerInvariant, SelectionProofs!MarkerInvariant,
      Selection!Bound, SelectionProofs!Bound,
      Selection!IsTransaction, SelectionProofs!IsTransaction,
      Selection!MarkerSpace, SelectionProofs!MarkerSpace,
      Selection!Selections, SelectionProofs!Selections,
      Selection!InitialTransactions, SelectionProofs!InitialTransactions,
      Selection!EmptyMarker, SelectionProofs!EmptyMarker, Selection!vars, selectionVars

\* INSTANCE also prefixes imported mathematical operators. Relate the finite
\* set predicates before discharging the instantiated module's assumptions.
THEOREM SelectionFiniteDomainsAgree ==
    \A values : Selection!IsFiniteSet(values) = SelectionProofs!IsFiniteSet(values)
  BY DEF Selection!IsFiniteSet, SelectionProofs!IsFiniteSet

THEOREM SelectionNextInvariantNamesAgree == Selection!Invariant' = SelectionProofs!Invariant'
  BY DEF Selection!Invariant, SelectionProofs!Invariant,
      Selection!TypeOK, SelectionProofs!TypeOK,
      Selection!ReservationInvariant, SelectionProofs!ReservationInvariant,
      Selection!CurrentInvariant, SelectionProofs!CurrentInvariant,
      Selection!MarkerInvariant, SelectionProofs!MarkerInvariant,
      Selection!Bound, SelectionProofs!Bound,
      Selection!IsTransaction, SelectionProofs!IsTransaction,
      Selection!MarkerSpace, SelectionProofs!MarkerSpace,
      Selection!Selections, SelectionProofs!Selections

THEOREM SelectionProofPremises ==
    SelectionProofs!Parameters /\ SelectionProofs!CorrectProtocol
  BY SMT, SelectionDomain, CorrectJournal, SelectionFiniteDomainsAgree
  DEF Selection!Parameters, SelectionProofs!Parameters, SelectionProofs!CorrectProtocol,
      Selection!Selections, SelectionProofs!Selections

THEOREM SelectionTransitionNamesAgree ==
  ASSUME NEW available, NEW clean, NEW admitted
  PROVE Selection!Next(available, clean, admitted) = SelectionProofs!Next(available, clean, admitted)
  BY DEF Selection!Next, SelectionProofs!Next,
      Selection!CoreNext, SelectionProofs!CoreNext,
      Selection!StorageNext, SelectionProofs!StorageNext,
      Selection!Reserve, SelectionProofs!Reserve,
      Selection!PublishLink, SelectionProofs!PublishLink,
      Selection!SealLink, SelectionProofs!SealLink,
      Selection!WriteMarker, SelectionProofs!WriteMarker,
      Selection!Flip, SelectionProofs!Flip,
      Selection!ClearMarker, SelectionProofs!ClearMarker,
      Selection!SealReservations, SelectionProofs!SealReservations,
      Selection!SealMarker, SelectionProofs!SealMarker,
      Selection!SealCurrent, SelectionProofs!SealCurrent,
      Selection!ProcessDeath, SelectionProofs!ProcessDeath,
      Selection!PowerLoss, SelectionProofs!PowerLoss,
      Selection!ForeignCurrent, SelectionProofs!ForeignCurrent,
      Selection!Selections, SelectionProofs!Selections,
      Selection!Bound, SelectionProofs!Bound,
      Selection!IsTransaction, SelectionProofs!IsTransaction,
      Selection!Marker, SelectionProofs!Marker,
      Selection!EmptyMarker, SelectionProofs!EmptyMarker,
      Selection!Classify, SelectionProofs!Classify,
      Selection!SameGeneration, SelectionProofs!SameGeneration

THEOREM JournalInitialSafety == Init => Invariant
<1> SUFFICES ASSUME Init PROVE Invariant
  OBVIOUS
<1>1. Selection!Invariant
  BY SMT, CorrectJournal, SelectionDomain, SelectionProofPremises, SelectionStateNamesAgree,
     SelectionProofs!SelectionInitialSafety DEF Init, SelectionProofs!CorrectProtocol
<1>2. Committed = FALSE
  BY DEF Init, Committed
<1>3. CellInvariantAll
  BY SMT, InitialCellSetSafety, <1>2 DEF Init, CellInvariantAll
<1>4. CachedEntriesEmpty /\ StableEntriesEmpty
  BY SMT, InitialCellFields, MissingProjection
  DEF Init, InitialCells, CachedEntriesEmpty, StableEntriesEmpty
<1>5. TypeOK
  BY SMT, <1>1, <1>3, SelectionDomain
  DEF Init, TypeOK, Selection!Init, Selection!Invariant, Selection!TypeOK,
      Selection!Selections, CellInvariantAll, CellInvariant, Modes
<1>6. EpochInvariant /\ MarkerCoversEntries /\ ControlInvariant
  BY SMT, <1>2, <1>4
  DEF Init, EpochInvariant, MarkerCoversEntries, ControlInvariant,
      Selection!Init, Selection!EmptyMarker, Classification
<1>7. QED
  BY <1>1, <1>3, <1>5, <1>6 DEF Invariant

THEOREM JournalSelectionProjection ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Next(available, admitted)
  PROVE [Selection!Next(available, TRUE, admitted)]_selectionVars
  BY SMT, CorrectJournal
  DEF Next, CoreNext, EnvironmentNext, BeginEpoch, PrepareSelection, WriteRunMarker,
      SealRunMarker, WriterStep, FinishWriting, FinishNoop, FlipSelection, SealCommittedSelection,
      ClassifyRecovery, RetryRecovery, RestoreStep, FinishRestoring, RemoveEntry, SealEntryRemoval,
      RemoveMarker, AlreadyMissingMarker, SealMarkerRemoval, CrashProcess, CrashPower,
      WritebackSelection, WritebackDestination, ExternalDestination, UnexpectedSelection,
      Selection!Next, Selection!CoreNext, Selection!StorageNext, Selection!ClearMarker

THEOREM JournalSelectionSafety ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted)
  PROVE Selection!Invariant'
<1>1. [Selection!Next(available, TRUE, admitted)]_selectionVars
  BY JournalSelectionProjection
<1>2. SelectionProofs!Invariant
  BY SMT, SelectionStateNamesAgree DEF Invariant
<1>3. CASE Selection!Next(available, TRUE, admitted)
  BY SMT, CorrectJournal, SelectionDomain, SelectionProofPremises, SelectionProofs!SelectionInduction,
     SelectionStateNamesAgree, SelectionNextInvariantNamesAgree, SelectionTransitionNamesAgree, <1>2, <1>3
  DEF SelectionProofs!CorrectProtocol
<1>4. CASE UNCHANGED selectionVars
  BY SMT, <1>4
  DEF Invariant, Selection!Invariant, Selection!TypeOK, Selection!ReservationInvariant,
      Selection!CurrentInvariant, Selection!MarkerInvariant, Selection!Bound, selectionVars
<1>5. QED BY <1>1, <1>3, <1>4

THEOREM DurableCommitCannotDisappearWithinEpoch ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted), Committed,
         epochTarget' = epochTarget
  PROVE Committed'
  BY SMT, CorrectJournal
  DEF Invariant, EpochInvariant, Committed, Next, CoreNext, EnvironmentNext,
      BeginEpoch, PrepareSelection, WriteRunMarker, SealRunMarker, WriterStep, FinishWriting,
      FinishNoop, FlipSelection, SealCommittedSelection, ClassifyRecovery, RetryRecovery,
      RestoreStep, FinishRestoring, RemoveEntry, SealEntryRemoval, RemoveMarker,
      AlreadyMissingMarker, SealMarkerRemoval, CrashProcess, CrashPower, WritebackSelection,
      WritebackDestination, ExternalDestination, UnexpectedSelection,
      Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations, selectionVars

THEOREM RejectedAdmissionHasNoCoreTransition ==
  ASSUME NEW available, NEW admitted, ~admitted, Next(available, admitted)
  PROVE EnvironmentNext(available)
  BY DEF Next

=============================================================================
