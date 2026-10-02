---- MODULE JournalMarkerProofs ----
EXTENDS JournalFrameProofs

THEOREM BeginEpochPreservesMarkerOrder ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds, Invariant, BeginEpoch(transaction, generation)
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF BeginEpoch, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM PrepareSelectionPreservesMarkerOrder ==
  ASSUME Invariant, PrepareSelection
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF PrepareSelection, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM WriteRunMarkerPreservesMarkerOrder ==
  ASSUME Invariant, WriteRunMarker
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF WriteRunMarker, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM SealRunMarkerPreservesMarkerOrder ==
  ASSUME Invariant, SealRunMarker
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF SealRunMarker, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM WriterStepPreservesMarkerOrder ==
  ASSUME NEW destination \in Destinations, Invariant, WriterStep(destination)
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF WriterStep, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM FinishWritingPreservesMarkerOrder ==
  ASSUME Invariant, FinishWriting
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF FinishWriting, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM FinishNoopPreservesMarkerOrder ==
  ASSUME Invariant, FinishNoop
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF FinishNoop, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM FlipSelectionPreservesMarkerOrder ==
  ASSUME NEW available \in SUBSET GenerationIds, Invariant, FlipSelection(available)
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF FlipSelection, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM SealCommittedSelectionPreservesMarkerOrder ==
  ASSUME Invariant, SealCommittedSelection
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF SealCommittedSelection, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM ClassifyRecoveryPreservesMarkerOrder ==
  ASSUME Invariant, ClassifyRecovery
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF ClassifyRecovery, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM RetryRecoveryPreservesMarkerOrder ==
  ASSUME Invariant, RetryRecovery
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF RetryRecovery, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM RestoreStepPreservesMarkerOrder ==
  ASSUME NEW destination \in Destinations, Invariant, RestoreStep(destination)
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF RestoreStep, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM FinishRestoringPreservesMarkerOrder ==
  ASSUME Invariant, FinishRestoring
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF FinishRestoring, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM RemoveEntryPreservesMarkerOrder ==
  ASSUME NEW destination \in Destinations, Invariant, RemoveEntry(destination)
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF RemoveEntry, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM SealEntryRemovalPreservesMarkerOrder ==
  ASSUME Invariant, SealEntryRemoval
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal
  DEF SealEntryRemoval, Invariant, TypeOK, CellSpace, DiskSpace, MarkerCoversEntries,
      CachedEntriesEmpty, StableEntriesEmpty, EntryWritebackImage, Selection!SealMarker

THEOREM RemoveMarkerPreservesMarkerOrder ==
  ASSUME Invariant, RemoveMarker
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF RemoveMarker, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM AlreadyMissingMarkerPreservesMarkerOrder ==
  ASSUME Invariant, AlreadyMissingMarker
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF AlreadyMissingMarker, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM SealMarkerRemovalPreservesMarkerOrder ==
  ASSUME Invariant, SealMarkerRemoval
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF SealMarkerRemoval, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM CrashProcessPreservesMarkerOrder ==
  ASSUME Invariant, CrashProcess
  PROVE MarkerCoversEntries'
  BY SMT
  DEF CrashProcess, Invariant, TypeOK, CellSpace, ControlSpace, MarkerCoversEntries,
      CachedEntriesEmpty, StableEntriesEmpty, ProcessDeathImage, Selection!ProcessDeath

THEOREM CrashPowerPreservesMarkerOrder ==
  ASSUME Invariant, CrashPower
  PROVE MarkerCoversEntries'
<1>1. (~markerVisible'.present \/ ~markerStable'.present) =>
        CachedEntriesEmpty /\ StableEntriesEmpty
  BY SMT DEF Invariant, MarkerCoversEntries, CrashPower, Selection!PowerLoss
<1>2. ASSUME ~markerVisible'.present \/ ~markerStable'.present
      PROVE CachedEntriesEmpty' /\ StableEntriesEmpty'
  <2>1. \A destination \in Destinations :
          ~cells'[destination].cached.entry.present /\ ~cells'[destination].durable.entry.present
    <3>1. TAKE destination \in Destinations
    <3>2. cells[destination] \in CellSpace /\ cells'[destination] \in PowerLossImages(cells[destination])
      BY SMT DEF Invariant, TypeOK, CrashPower
    <3>3. QED
      BY SMT, <1>1, <1>2, <3>2, PowerLossPreservesAbsentEntries
      DEF CachedEntriesEmpty, StableEntriesEmpty
  <2>2. QED
    BY <2>1 DEF CachedEntriesEmpty, StableEntriesEmpty
<1>3. QED
  BY <1>2 DEF MarkerCoversEntries

THEOREM WritebackSelectionPreservesMarkerOrder ==
  ASSUME Invariant, WritebackSelection
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF WritebackSelection, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

THEOREM WritebackDestinationPreservesMarkerOrder ==
  ASSUME NEW destination \in Destinations, Invariant, WritebackDestination(destination)
  PROVE MarkerCoversEntries'
  BY SMT, ReplacementFrame
  DEF WritebackDestination, Invariant, TypeOK, CellSpace, DiskSpace, MarkerCoversEntries,
      CachedEntriesEmpty, StableEntriesEmpty, EntryWritebackImage, LiveWritebackImage, ReplaceCell, selectionVars

THEOREM ExternalDestinationPreservesMarkerOrder ==
  ASSUME NEW destination \in Destinations, NEW value \in Objects, Invariant, ExternalDestination(destination, value)
  PROVE MarkerCoversEntries'
  BY SMT, ReplacementFrame
  DEF ExternalDestination, Invariant, TypeOK, CellSpace, DiskSpace, MarkerCoversEntries,
      CachedEntriesEmpty, StableEntriesEmpty, ExternalEditImage, ReplaceCell, selectionVars

THEOREM UnexpectedSelectionPreservesMarkerOrder ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW selection \in Selection!Selections, Invariant, UnexpectedSelection(available, selection)
  PROVE MarkerCoversEntries'
  BY SMT, CorrectJournal, ReplacementFrame, MissingProjection, CellImagesRemainTyped
  DEF UnexpectedSelection, Invariant, TypeOK, MarkerCoversEntries, ControlInvariant, CachedEntriesEmpty,
      StableEntriesEmpty, WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination, Restore,
      SealPrior, KeepForeign, DeleteEntry, ExternalEdit, InitialCell, InitialDisk, CellImages, PowerLossImages,
      ReplaceCell, Selection!Reserve, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!Marker, Selection!SealMarker, Selection!Flip, Selection!SealCurrent, Selection!ClearMarker,
      Selection!EmptyMarker, Selection!ProcessDeath, Selection!PowerLoss, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, control

=============================================================================
