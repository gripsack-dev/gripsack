---- MODULE JournalMetadataProofs ----
EXTENDS JournalControlProofs

THEOREM JournalTypesPreserved ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted)
  PROVE TypeOK'
  BY SMT, JournalSelectionSafety, JournalCellShapesPreserved, SelectionDomain
  DEF Invariant, TypeOK, Modes, Next, CoreNext, EnvironmentNext, BeginEpoch,
      PrepareSelection, WriteRunMarker, SealRunMarker, WriterStep, FinishWriting, FinishNoop,
      FlipSelection, SealCommittedSelection, ClassifyRecovery, RetryRecovery, RestoreStep,
      FinishRestoring, RemoveEntry, SealEntryRemoval, RemoveMarker, AlreadyMissingMarker,
      SealMarkerRemoval, CrashProcess, CrashPower, WritebackSelection, WritebackDestination,
      ExternalDestination, UnexpectedSelection, Selection!Invariant, Selection!TypeOK,
      Selection!Selections, Selection!Parameters, control

THEOREM JournalMarkerOrdersEvidence ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted)
  PROVE MarkerCoversEntries'
  BY SMT, BeginEpochPreservesMarkerOrder, PrepareSelectionPreservesMarkerOrder,
     WriteRunMarkerPreservesMarkerOrder, SealRunMarkerPreservesMarkerOrder,
     WriterStepPreservesMarkerOrder, FinishWritingPreservesMarkerOrder, FinishNoopPreservesMarkerOrder,
     FlipSelectionPreservesMarkerOrder, SealCommittedSelectionPreservesMarkerOrder,
     ClassifyRecoveryPreservesMarkerOrder, RetryRecoveryPreservesMarkerOrder, RestoreStepPreservesMarkerOrder,
     FinishRestoringPreservesMarkerOrder, RemoveEntryPreservesMarkerOrder, SealEntryRemovalPreservesMarkerOrder,
     RemoveMarkerPreservesMarkerOrder, AlreadyMissingMarkerPreservesMarkerOrder, SealMarkerRemovalPreservesMarkerOrder,
     CrashProcessPreservesMarkerOrder, CrashPowerPreservesMarkerOrder, WritebackSelectionPreservesMarkerOrder,
     WritebackDestinationPreservesMarkerOrder, ExternalDestinationPreservesMarkerOrder, UnexpectedSelectionPreservesMarkerOrder
  DEF Next, CoreNext, EnvironmentNext

THEOREM JournalEpochIdentityPreserved ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted)
  PROVE EpochInvariant'
  BY SMT, BeginEpochPreservesEpoch, PrepareSelectionPreservesEpoch, WriteRunMarkerPreservesEpoch,
     SealRunMarkerPreservesEpoch, WriterStepPreservesEpoch, FinishWritingPreservesEpoch, FinishNoopPreservesEpoch,
     FlipSelectionPreservesEpoch, SealCommittedSelectionPreservesEpoch, ClassifyRecoveryPreservesEpoch,
     RetryRecoveryPreservesEpoch, RestoreStepPreservesEpoch, FinishRestoringPreservesEpoch, RemoveEntryPreservesEpoch,
     SealEntryRemovalPreservesEpoch, RemoveMarkerPreservesEpoch, AlreadyMissingMarkerPreservesEpoch,
     SealMarkerRemovalPreservesEpoch, CrashProcessPreservesEpoch, CrashPowerPreservesEpoch,
     WritebackSelectionPreservesEpoch, WritebackDestinationPreservesEpoch, ExternalDestinationPreservesEpoch,
     UnexpectedSelectionPreservesEpoch
  DEF Next, CoreNext, EnvironmentNext

THEOREM JournalControlPreserved ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted)
  PROVE ControlInvariant'
  BY SMT, BeginEpochPreservesControl, PrepareSelectionPreservesControl, WriteRunMarkerPreservesControl,
     SealRunMarkerPreservesControl, WriterStepPreservesControl, FinishWritingPreservesControl, FinishNoopPreservesControl,
     FlipSelectionPreservesControl, SealCommittedSelectionPreservesControl, ClassifyRecoveryPreservesControl,
     RetryRecoveryPreservesControl, RestoreStepPreservesControl, FinishRestoringPreservesControl, RemoveEntryPreservesControl,
     SealEntryRemovalPreservesControl, RemoveMarkerPreservesControl, AlreadyMissingMarkerPreservesControl,
     SealMarkerRemovalPreservesControl, CrashProcessPreservesControl, CrashPowerPreservesControl,
     WritebackSelectionPreservesControl, WritebackDestinationPreservesControl, ExternalDestinationPreservesControl,
     UnexpectedSelectionPreservesControl
  DEF Next, CoreNext, EnvironmentNext

THEOREM JournalInduction ==
  ASSUME NEW available \in SUBSET GenerationIds, NEW admitted \in BOOLEAN,
         Invariant, Next(available, admitted)
  PROVE Invariant'
  BY JournalTypesPreserved, JournalSelectionSafety, JournalDestinationsPreserveUndo,
     JournalMarkerOrdersEvidence, JournalEpochIdentityPreserved, JournalControlPreserved
  DEF Invariant

THEOREM GeneralJournalSafety == Spec => []Invariant
<1>1. Init => Invariant
  BY JournalInitialSafety
<1>2. Invariant /\ [Next(AvailableGenerations, TRUE)]_vars => Invariant'
  BY SMT, JournalInduction, SelectionDomain
  DEF Invariant, TypeOK, CellInvariantAll, EpochInvariant, MarkerCoversEntries, ControlInvariant,
      CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification, vars, selectionVars, control,
      Selection!Invariant, Selection!TypeOK, Selection!ReservationInvariant, Selection!CurrentInvariant,
      Selection!MarkerInvariant, Selection!Bound
<1>3. QED BY PTL, <1>1, <1>2 DEF Spec

THEOREM JournalInvariantPreservesRecoveryEvidence ==
  Invariant => RecoveryEvidencePreserved /\ MutationHasDurableMarker
  BY SMT DEF Invariant, CellInvariantAll, CellInvariant, CellState, UndoEvidence, Covered,
      ControlInvariant, RecoveryEvidencePreserved, MutationHasDurableMarker

THEOREM EntryRemovalRequiresDurableRestoration ==
  ASSUME NEW destination \in Destinations, Invariant, ~Committed, RemoveEntry(destination)
  PROVE ~UnsafeOwned(cells[destination], cells[destination].durable.live)
<1>1. ~CleanupCommitted /\ cells[destination].control.processed
  BY SMT, CleanupGrantMatchesExactCommit DEF RemoveEntry, DeleteEntry
<1>2. QED
  BY SMT, <1>1 DEF Invariant, CellInvariantAll, CellInvariant, CellState

THEOREM CorruptAdmissionRetainsCoreAuthority ==
  ASSUME NEW available, Next(available, FALSE)
  PROVE EnvironmentNext(available)
  BY RejectedAdmissionHasNoCoreTransition

THEOREM RecoverySafetyWithoutCrashBound == Spec =>
    [](RecoveryEvidencePreserved /\ MutationHasDurableMarker)
  BY PTL, GeneralJournalSafety, JournalInvariantPreservesRecoveryEvidence

=============================================================================
