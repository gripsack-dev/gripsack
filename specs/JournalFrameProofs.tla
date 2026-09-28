---- MODULE JournalFrameProofs ----
EXTENDS JournalCellProofs

THEOREM EpochStateFrame ==
  ASSUME UNCHANGED <<markerVisible, markerStable, pending, currentVisible, currentStable,
                    epochPrevious, epochTarget, cells>>
  PROVE EpochInvariant' = EpochInvariant
  BY SMT DEF EpochInvariant, Committed, StableEntriesEmpty, Selection!Marker, Selection!IsTransaction

THEOREM RetryRecoveryEpochFrame ==
  ASSUME Invariant, RetryRecovery
  PROVE EpochInvariant'
  BY SMT, EpochStateFrame DEF Invariant, RetryRecovery, selectionVars

THEOREM MarkerStateFrame ==
  ASSUME UNCHANGED <<markerVisible, markerStable, cells>>
  PROVE MarkerCoversEntries' = MarkerCoversEntries
  BY SMT DEF MarkerCoversEntries, CachedEntriesEmpty, StableEntriesEmpty

EpochMetadata ==
    /\ (markerVisible.present => markerVisible = Selection!Marker(epochPrevious, epochTarget))
    /\ (markerStable.present => markerStable = Selection!Marker(epochPrevious, epochTarget))
    /\ (pending # NoSelection => pending = epochTarget)
    /\ (epochTarget # NoSelection => Selection!IsTransaction(epochTarget))
    /\ (Committed => currentVisible = epochTarget)

THEOREM EpochMetadataProjection ==
  EpochInvariant => EpochMetadata
  BY SMT DEF EpochInvariant, EpochMetadata

THEOREM EpochMetadataReconstruction ==
  EpochMetadata /\ MarkerCoversEntries => EpochInvariant
  BY SMT DEF EpochMetadata, EpochInvariant, MarkerCoversEntries

THEOREM EpochMetadataNextReconstruction ==
  EpochMetadata' /\ MarkerCoversEntries' => EpochInvariant'
  BY SMT DEF EpochMetadata, EpochInvariant, MarkerCoversEntries

THEOREM EpochMetadataFrame ==
  ASSUME UNCHANGED <<markerVisible, markerStable, pending, currentVisible, currentStable,
                    epochPrevious, epochTarget>>
  PROVE EpochMetadata' = EpochMetadata
  BY SMT DEF EpochMetadata, Committed, Selection!Marker, Selection!IsTransaction

THEOREM PowerLossPreservesAbsentEntries ==
  ASSUME NEW c \in CellSpace, ~c.cached.entry.present, ~c.durable.entry.present,
         NEW next \in PowerLossImages(c)
  PROVE ~next.cached.entry.present /\ ~next.durable.entry.present
  BY SMT DEF PowerLossImages, PowerLossImage, CellSpace, DiskSpace, ControlSpace, EntrySpace

=============================================================================
