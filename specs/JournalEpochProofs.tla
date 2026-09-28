---- MODULE JournalEpochProofs ----
EXTENDS JournalMarkerProofs

THEOREM BeginEpochPreservesEpoch ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds, Invariant, BeginEpoch(transaction, generation)
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection, SelectionDomain
  DEF BeginEpoch, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!Reserve, Selection!Invariant, Selection!TypeOK, Selection!ReservationInvariant, Selection!CurrentInvariant, Selection!Bound, Selection!Parameters, Selection!Selections, ControlInvariant, CachedEntriesEmpty, StableEntriesEmpty, selectionVars, control
<1>2. MarkerCoversEntries'
  BY BeginEpochPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM PrepareSelectionPreservesEpoch ==
  ASSUME Invariant, PrepareSelection
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF PrepareSelection, Invariant, Selection!PublishLink, Selection!SealLink, selectionVars, control
<1>2. MarkerCoversEntries'
  BY PrepareSelectionPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM WriteRunMarkerPreservesEpoch ==
  ASSUME Invariant, WriteRunMarker
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF WriteRunMarker, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!WriteMarker, ControlInvariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY WriteRunMarkerPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM SealRunMarkerPreservesEpoch ==
  ASSUME Invariant, SealRunMarker
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF SealRunMarker, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!SealMarker, selectionVars, control
<1>2. MarkerCoversEntries'
  BY SealRunMarkerPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM WriterStepPreservesEpoch ==
  ASSUME NEW destination \in Destinations, Invariant, WriterStep(destination)
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF WriterStep, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY WriterStepPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM FinishWritingPreservesEpoch ==
  ASSUME Invariant, FinishWriting
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF FinishWriting, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY FinishWritingPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM FinishNoopPreservesEpoch ==
  ASSUME Invariant, FinishNoop
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF FinishNoop, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY FinishNoopPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM FlipSelectionPreservesEpoch ==
  ASSUME NEW available \in SUBSET GenerationIds, Invariant, FlipSelection(available)
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF FlipSelection, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!Flip, selectionVars, control
<1>2. MarkerCoversEntries'
  BY FlipSelectionPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM SealCommittedSelectionPreservesEpoch ==
  ASSUME Invariant, SealCommittedSelection
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF SealCommittedSelection, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!SealCurrent, selectionVars, control
<1>2. MarkerCoversEntries'
  BY SealCommittedSelectionPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM ClassifyRecoveryPreservesEpoch ==
  ASSUME Invariant, ClassifyRecovery
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF ClassifyRecovery, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY ClassifyRecoveryPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM RetryRecoveryPreservesEpoch ==
  ASSUME Invariant, RetryRecovery
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF RetryRecovery, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY RetryRecoveryPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM RestoreStepPreservesEpoch ==
  ASSUME NEW destination \in Destinations, Invariant, RestoreStep(destination)
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF RestoreStep, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY RestoreStepPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM FinishRestoringPreservesEpoch ==
  ASSUME Invariant, FinishRestoring
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF FinishRestoring, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY FinishRestoringPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM RemoveEntryPreservesEpoch ==
  ASSUME NEW destination \in Destinations, Invariant, RemoveEntry(destination)
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF RemoveEntry, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY RemoveEntryPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM SealEntryRemovalPreservesEpoch ==
  ASSUME Invariant, SealEntryRemoval
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF SealEntryRemoval, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!SealMarker, selectionVars, control
<1>2. MarkerCoversEntries'
  BY SealEntryRemovalPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM RemoveMarkerPreservesEpoch ==
  ASSUME Invariant, RemoveMarker
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF RemoveMarker, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!ClearMarker, Selection!EmptyMarker, selectionVars, control
<1>2. MarkerCoversEntries'
  BY RemoveMarkerPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM AlreadyMissingMarkerPreservesEpoch ==
  ASSUME Invariant, AlreadyMissingMarker
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF AlreadyMissingMarker, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY AlreadyMissingMarkerPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM SealMarkerRemovalPreservesEpoch ==
  ASSUME Invariant, SealMarkerRemoval
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF SealMarkerRemoval, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!SealMarker, selectionVars, control
<1>2. MarkerCoversEntries'
  BY SealMarkerRemovalPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM CrashProcessPreservesEpoch ==
  ASSUME Invariant, CrashProcess
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF CrashProcess, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!ProcessDeath, selectionVars, control
<1>2. MarkerCoversEntries'
  BY CrashProcessPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM CrashPowerPreservesEpoch ==
  ASSUME Invariant, CrashPower
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF CrashPower, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!PowerLoss, selectionVars, control
<1>2. MarkerCoversEntries'
  BY CrashPowerPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM WritebackSelectionPreservesEpoch ==
  ASSUME Invariant, WritebackSelection
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF WritebackSelection, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!SealReservations, Selection!SealMarker, Selection!SealCurrent, selectionVars, control
<1>2. MarkerCoversEntries'
  BY WritebackSelectionPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM WritebackDestinationPreservesEpoch ==
  ASSUME NEW destination \in Destinations, Invariant, WritebackDestination(destination)
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF WritebackDestination, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY WritebackDestinationPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM ExternalDestinationPreservesEpoch ==
  ASSUME NEW destination \in Destinations, NEW value \in Objects, Invariant, ExternalDestination(destination, value)
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, EpochMetadataProjection, EpochMetadataFrame
  DEF ExternalDestination, Invariant, selectionVars, control
<1>2. MarkerCoversEntries'
  BY ExternalDestinationPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

THEOREM UnexpectedSelectionPreservesEpoch ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW selection \in Selection!Selections, Invariant, UnexpectedSelection(available, selection)
  PROVE EpochInvariant'
<1>1. EpochMetadata'
  BY SMT, CorrectJournal, EpochMetadataProjection
  DEF UnexpectedSelection, Invariant, EpochInvariant, EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction, Selection!ForeignCurrent, selectionVars, control
<1>2. MarkerCoversEntries'
  BY UnexpectedSelectionPreservesMarkerOrder
<1>3. QED
  BY <1>1, <1>2, EpochMetadataNextReconstruction

=============================================================================
