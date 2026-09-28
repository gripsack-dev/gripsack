---- MODULE JournalControlProofs ----
EXTENDS JournalControlFrames

THEOREM BeginEpochPreservesControl ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds, Invariant, BeginEpoch(transaction, generation)
  PROVE ControlInvariant'
  BY SMT, CorrectJournal, SelectionDomain, InitialCellFields, MissingProjection
  DEF BeginEpoch, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, TypeOK, CellSpace, DiskSpace, ControlSpace, Selection!Reserve, Selection!Invariant, Selection!ReservationInvariant, Selection!CurrentInvariant, Selection!Bound, Selection!TypeOK, Selection!Parameters, Selection!Selections, Selection!IsTransaction, InitialCell, InitialDisk, Missing

THEOREM PrepareSelectionPreservesControl ==
  ASSUME Invariant, PrepareSelection
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF PrepareSelection, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, Selection!PublishLink, Selection!SealLink

THEOREM WriteRunMarkerPreservesControl ==
  ASSUME Invariant, WriteRunMarker
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF WriteRunMarker, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, Selection!WriteMarker

THEOREM SealRunMarkerPreservesControl ==
  ASSUME Invariant, SealRunMarker
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF SealRunMarker, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, Selection!SealMarker

THEOREM WriterStepPreservesControl ==
  ASSUME NEW destination \in Destinations, Invariant, WriterStep(destination)
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF WriterStep, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control

THEOREM FinishWritingPreservesControl ==
  ASSUME Invariant, FinishWriting
  PROVE ControlInvariant'
  BY SMT, CorrectJournal, ProcessDeathSetsRecoveryStage
  DEF FinishWriting, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, TypeOK, CellSpace, DiskSpace, ControlSpace, ProcessDeathImage

THEOREM FinishNoopPreservesControl ==
  ASSUME Invariant, FinishNoop
  PROVE ControlInvariant'
  BY SMT, CorrectJournal, ProcessDeathSetsRecoveryStage
  DEF FinishNoop, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, TypeOK, CellSpace, DiskSpace, ControlSpace, ProcessDeathImage

THEOREM FlipSelectionPreservesControl ==
  ASSUME NEW available \in SUBSET GenerationIds, Invariant, FlipSelection(available)
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF FlipSelection, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, Selection!Flip

THEOREM SealCommittedSelectionPreservesControl ==
  ASSUME Invariant, SealCommittedSelection
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF SealCommittedSelection, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, Selection!SealCurrent

THEOREM ClassifyRecoveryPreservesControl ==
  ASSUME Invariant, ClassifyRecovery
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF ClassifyRecovery, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, Selection!Invariant, Selection!MarkerInvariant

THEOREM RetryRecoveryPreservesControl ==
  ASSUME Invariant, RetryRecovery
  PROVE ControlInvariant'
<1>1. mode' = "recovering" /\
        \A destination \in Destinations : cells'[destination].control.stage = "recover"
  BY SMT DEF Invariant, ControlInvariant, RetryRecovery
<1>2. QED
  BY ONLY SMT, <1>1 DEF ControlInvariant

THEOREM RestoreStepPreservesControl ==
  ASSUME NEW destination \in Destinations, Invariant, RestoreStep(destination)
  PROVE ControlInvariant'
  BY SMT, CorrectJournal, RecoveryImagesKeepStage, ReplacementFrame
  DEF RestoreStep, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, TypeOK, CellSpace, DiskSpace, ControlSpace, Restore, SealPrior, KeepForeign, RestorationImage, PriorSealImage, KeepImage

THEOREM FinishRestoringPreservesControl ==
  ASSUME Invariant, FinishRestoring
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF FinishRestoring, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control

THEOREM RemoveEntryPreservesControl ==
  ASSUME NEW destination \in Destinations, Invariant, RemoveEntry(destination)
  PROVE ControlInvariant'
  BY SMT, CorrectJournal, RecoveryImagesKeepStage, ReplacementFrame
  DEF RemoveEntry, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, TypeOK, CellSpace, DiskSpace, ControlSpace, DeleteEntry, EntryRemovalImage

THEOREM SealEntryRemovalPreservesControl ==
  ASSUME Invariant, SealEntryRemoval
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF SealEntryRemoval, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, TypeOK, CellSpace, DiskSpace, ControlSpace, Selection!SealMarker, EntryWritebackImage

THEOREM RemoveMarkerPreservesControl ==
  ASSUME Invariant, RemoveMarker
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF RemoveMarker, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, Selection!ClearMarker, Selection!EmptyMarker

THEOREM AlreadyMissingMarkerPreservesControl ==
  ASSUME Invariant, AlreadyMissingMarker
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF AlreadyMissingMarker, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control

THEOREM SealMarkerRemovalPreservesControl ==
  ASSUME Invariant, SealMarkerRemoval
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF SealMarkerRemoval, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, Selection!SealMarker

THEOREM CrashProcessPreservesControl ==
  ASSUME Invariant, CrashProcess
  PROVE ControlInvariant'
<1>1. mode' = "recovering" /\
        \A destination \in Destinations : cells'[destination].control.stage = "recover"
  BY SMT, ProcessDeathSetsRecoveryStage DEF Invariant, TypeOK, CrashProcess
<1>2. QED
  BY ONLY SMT, <1>1 DEF ControlInvariant

THEOREM CrashPowerPreservesControl ==
  ASSUME Invariant, CrashPower
  PROVE ControlInvariant'
<1>1. mode' = "recovering" /\
        \A destination \in Destinations : cells'[destination].control.stage = "recover"
  BY SMT, PowerLossSetsRecoveryStage DEF Invariant, TypeOK, CrashPower
<1>2. QED
  BY ONLY SMT, <1>1 DEF ControlInvariant

THEOREM WritebackSelectionPreservesControl ==
  ASSUME Invariant, WritebackSelection
  PROVE ControlInvariant'
  BY SMT, CorrectJournal
  DEF WritebackSelection, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries,
      CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify,
      Selection!Marker, selectionVars, control, Selection!SealReservations, Selection!SealMarker,
      Selection!SealCurrent, Selection!Invariant, Selection!MarkerInvariant, Selection!ReservationInvariant

THEOREM WritebackDestinationPreservesControl ==
  ASSUME NEW destination \in Destinations, Invariant, WritebackDestination(destination)
  PROVE ControlInvariant'
  BY SMT, CorrectJournal, WritebackKeepsStage, ReplacementFrame
  DEF WritebackDestination, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, TypeOK, CellSpace, DiskSpace, ControlSpace, EntryWritebackImage, LiveWritebackImage

THEOREM ExternalDestinationPreservesControl ==
  ASSUME NEW destination \in Destinations, NEW value \in Objects, Invariant, ExternalDestination(destination, value)
  PROVE ControlInvariant'
  BY SMT, CorrectJournal, ExternalEditKeepsEntryControl, ReplacementFrame
  DEF ExternalDestination, Invariant, ControlInvariant, EpochInvariant, MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, Selection!Classify, Selection!Marker, selectionVars, control, TypeOK, CellSpace, DiskSpace, ControlSpace, ExternalEditImage

THEOREM UnexpectedSelectionPreservesControl ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW selection \in Selection!Selections, Invariant, UnexpectedSelection(available, selection)
  PROVE ControlInvariant'
<1>1. mode' \in {"recovering", "refused"} /\
        \A destination \in Destinations : cells'[destination].control.stage = "recover"
  BY SMT DEF Invariant, ControlInvariant, UnexpectedSelection, control
<1>2. QED
  BY ONLY SMT, <1>1 DEF ControlInvariant

=============================================================================
