---- MODULE JournalCellProofs ----
EXTENDS JournalLifecycleProofs

THEOREM JournalCellShapesPreserved ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted)
  PROVE cells' \in [Destinations -> CellSpace]
<1>1. cells \in [Destinations -> CellSpace]
  BY DEF Invariant, TypeOK
<1>2. \A destination \in Destinations : CellImages(cells[destination]) \subseteq CellSpace
  BY SMT, CellImagesRemainTyped, <1>1
<1>3. CASE \E transaction \in Transactions, generation \in GenerationIds : BeginEpoch(transaction, generation)
  BY SMT, InitialCellType, CellValueTypes, <1>1, <1>3 DEF BeginEpoch
<1>4. CASE \E destination \in Destinations : \E next \in CellImages(cells[destination]) :
            cells' = [cells EXCEPT ![destination] = next]
  BY SMT, <1>1, <1>2, <1>4, ReplacementDomain DEF ReplaceCell
<1>5. CASE cells' = [destination \in Destinations |-> ProcessDeathImage(cells[destination])]
  <2>1. \A destination \in Destinations : ProcessDeathImage(cells[destination]) \in CellSpace
    <3> TAKE destination \in Destinations
    <3>1. ProcessDeathImage(cells[destination]) \in CellImages(cells[destination])
      BY DEF CellImages
    <3>2. QED BY ONLY SMT, <3>1, <1>2
  <2>2. QED BY ONLY SMT, <2>1, <1>5
<1>6. CASE cells' = [destination \in Destinations |-> EntryWritebackImage(cells[destination])]
  <2>1. \A destination \in Destinations : EntryWritebackImage(cells[destination]) \in CellSpace
    <3> TAKE destination \in Destinations
    <3>1. EntryWritebackImage(cells[destination]) \in CellImages(cells[destination])
      BY DEF CellImages
    <3>2. QED BY ONLY SMT, <3>1, <1>2
  <2>2. QED BY ONLY SMT, <2>1, <1>6
<1>7. CASE CrashPower
  BY SMT, <1>2, <1>7 DEF CrashPower, CellImages
<1>8. CASE UNCHANGED cells
  BY <1>1, <1>8
<1>9. QED
  BY SMT, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8
  DEF Next, CoreNext, EnvironmentNext, PrepareSelection, WriteRunMarker, SealRunMarker,
      WriterStep, FinishWriting, FinishNoop, FlipSelection, SealCommittedSelection,
      ClassifyRecovery, RetryRecovery, RestoreStep, FinishRestoring, RemoveEntry,
      SealEntryRemoval, RemoveMarker, AlreadyMissingMarker, SealMarkerRemoval,
      CrashProcess, WritebackSelection, WritebackDestination, ExternalDestination,
      UnexpectedSelection, CellImages

THEOREM CleanupGrantMatchesExactCommit ==
  ASSUME NEW destination \in Destinations, Invariant, mode = "cleanup",
         cells[destination].cached.entry.present
  PROVE CleanupCommitted = Committed
  BY SMT, CorrectJournal
  DEF Invariant, EpochInvariant, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      CleanupCommitted, Committed, Classification, Selection!Classify, Selection!Invariant,
      Selection!MarkerInvariant, Selection!Marker, Selection!IsTransaction

THEOREM NoRestorationOfCommittedEntries ==
  ASSUME NEW destination \in Destinations, Invariant, RestoreStep(destination)
  PROVE ~Committed
  BY SMT
  DEF Invariant, ControlInvariant, RestoreStep, Restore, SealPrior, KeepForeign, CachedEntriesEmpty

THEOREM NonBeginningStepIsCellwise ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted),
         ~(\E transaction \in Transactions, generation \in GenerationIds : BeginEpoch(transaction, generation))
  PROVE \A destination \in Destinations :
          cells'[destination] = cells[destination] \/ CellStep(cells[destination], cells'[destination], Committed)
  BY SMT, CorrectJournal, CleanupGrantMatchesExactCommit, NoRestorationOfCommittedEntries,
     ReplacementFrame
  DEF Invariant, TypeOK, Next, CoreNext, EnvironmentNext, PrepareSelection, WriteRunMarker,
      SealRunMarker, WriterStep, FinishWriting, FinishNoop, FlipSelection, SealCommittedSelection,
      ClassifyRecovery, RetryRecovery, RestoreStep, FinishRestoring, RemoveEntry, SealEntryRemoval,
      RemoveMarker, AlreadyMissingMarker, SealMarkerRemoval, CrashProcess, CrashPower,
      WritebackSelection, WritebackDestination, ExternalDestination, UnexpectedSelection,
      CellStep, CoreCellStep, ProcessDeath, PowerLoss, PowerLossImages,
      WritebackEntry, WritebackLive, DeleteEntry, ReplaceCell

THEOREM NonBeginningStepKeepsEpoch ==
  ASSUME NEW available, NEW admitted, Next(available, admitted),
         ~(\E transaction \in Transactions, generation \in GenerationIds : BeginEpoch(transaction, generation))
  PROVE epochTarget' = epochTarget
  BY SMT
  DEF Next, CoreNext, EnvironmentNext, PrepareSelection, WriteRunMarker, SealRunMarker,
      WriterStep, FinishWriting, FinishNoop, FlipSelection, SealCommittedSelection, ClassifyRecovery,
      RetryRecovery, RestoreStep, FinishRestoring, RemoveEntry, SealEntryRemoval, RemoveMarker,
      AlreadyMissingMarker, SealMarkerRemoval, CrashProcess, CrashPower, WritebackSelection,
      WritebackDestination, ExternalDestination, UnexpectedSelection, control

THEOREM JournalCellSafetyWithinEpoch ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted),
         ~(\E transaction \in Transactions, generation \in GenerationIds : BeginEpoch(transaction, generation))
  PROVE CellInvariantAll'
<1>1. cells' \in [Destinations -> CellSpace]
  BY JournalCellShapesPreserved
<1>2. \A destination \in Destinations :
        cells'[destination] = cells[destination] \/ CellStep(cells[destination], cells'[destination], Committed)
  BY NonBeginningStepIsCellwise
<1>3. Committed => Committed'
  BY SMT, NonBeginningStepKeepsEpoch, DurableCommitCannotDisappearWithinEpoch
<1>4. QED
  BY SMT, PointwiseCellPreservation, <1>1, <1>2, <1>3
  DEF Invariant, TypeOK, CellInvariantAll, Committed

THEOREM NewEpochHasFreshUndoSnapshots ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds,
         Invariant, BeginEpoch(transaction, generation)
  PROVE CellInvariantAll'
<1>1. Next(AvailableGenerations, TRUE)
  BY DEF Next, CoreNext
<1>2. AvailableGenerations \in SUBSET GenerationIds
  BY SelectionDomain DEF Selection!Parameters
<1>3. Selection!Invariant'
  BY JournalSelectionSafety, <1>1, <1>2
<1>4. ~Committed'
  BY SMT, <1>3
  DEF BeginEpoch, Selection!Reserve, Selection!Invariant, Selection!ReservationInvariant, Committed
<1>5. \A destination \in Destinations :
        /\ cells[destination].cached.live.value \in Objects
        /\ cells[destination].durable.live.value \in Objects
  BY SMT, CellValueTypes DEF Invariant, TypeOK
<1>6. QED
  BY SMT, InitialCellSafety, <1>4, <1>5
  DEF BeginEpoch, CellInvariantAll, Committed

THEOREM JournalDestinationsPreserveUndo ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted)
  PROVE CellInvariantAll'
  BY SMT, JournalCellSafetyWithinEpoch, NewEpochHasFreshUndoSnapshots

=============================================================================
