---- MODULE LifecycleRetentionControlProofs ----
EXTENDS LifecycleRetentionSteps

THEOREM MetadataEnvironmentKeepsCollectionInputs ==
  MetadataEnvironment => UNCHANGED <<visible, waterVisible, publicationPhase, activationPhase,
      preparationOwner, markerVisible, currentVisible, pendingC>>
  BY SMT DEF MetadataEnvironment, PublicationWithHooks, PublicationStep,
      Publication!WritebackWater, Publication!WritebackGenerations, StationaryJournalStep,
      Selection!SealReservations, Selection!SealMarker, WritebackDestination, ExternalDestination,
      WritebackLifecycleCurrent, JournalHomeBarrier, Selection!SealCurrent, HookCurrentWriteback,
      ActivationStorageStep, Hooks!WritebackCurrent, Hooks!WritebackPending,
      Hooks!WritebackOutcomes, Hooks!WritebackArchive, Hooks!home, Hooks!process,
      publicationVars, lifecycleVars, vars, selectionVars, hookVars, preparationHistoryVars

THEOREM MetadataEnvironmentProjectsPublicationWriteback ==
  MetadataEnvironment => [Publication!WritebackWater \/ Publication!WritebackGenerations]_publicationVars
  BY SMT DEF MetadataEnvironment, StationaryJournalStep, WritebackLifecycleCurrent,
      ActivationStorageStep, lifecycleVars

THEOREM MetadataEnvironmentKeepsCurrentBarrier ==
  MetadataEnvironment => currentStable' \in {currentStable, currentVisible}
  BY SMT DEF MetadataEnvironment, PublicationWithHooks, PublicationStep, StationaryJournalStep,
      Selection!SealReservations, Selection!SealMarker, WritebackDestination, ExternalDestination,
      WritebackLifecycleCurrent, JournalHomeBarrier, Selection!SealCurrent, ActivationStorageStep,
      lifecycleVars, vars, selectionVars

THEOREM MetadataEnvironmentKeepsCollectionBarriers ==
  ASSUME RetentionLifecycleInvariant, MetadataEnvironment
  PROVE /\ waterStable <= waterStable'
        /\ currentStable' \in {currentStable, currentVisible}
        /\ (stable = visible => stable' = visible)
<1>1. waterVisible \in Int /\ waterStable \in Int /\ waterStable <= waterVisible /\
       visible \in SUBSET GenerationIds /\ stable \in SUBSET GenerationIds
  BY SMT, PublicationDomain, CompositionDomain
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant,
      Publication!Invariant, Publication!TypeOK, Publication!StorageInvariant,
      Publication!GenerationIds, Publication!Parameters
<1>2. [Publication!WritebackWater \/ Publication!WritebackGenerations]_publicationVars
  BY MetadataEnvironmentProjectsPublicationWriteback
<1>3. waterStable <= waterStable'
  BY ONLY SMT, <1>1, <1>2
  DEF Publication!WritebackWater, Publication!WritebackGenerations, publicationVars
<1>4. stable = visible => stable' = visible
  <2> SUFFICES ASSUME stable = visible PROVE stable' = visible OBVIOUS
  <2>1. stable' \subseteq visible
    BY ONLY SMT, <1>2, stable = visible
    DEF Publication!WritebackWater, Publication!WritebackGenerations, publicationVars
  <2>2. visible \subseteq stable'
    BY ONLY SMT, <1>2, stable = visible
    DEF Publication!WritebackWater, Publication!WritebackGenerations, publicationVars
  <2>3. QED BY ONLY SMT, <2>1, <2>2
<1>5. currentStable' \in {currentStable, currentVisible}
  BY MetadataEnvironmentKeepsCurrentBarrier
<1>6. QED BY ONLY SMT, <1>3, <1>4, <1>5

THEOREM MetadataEnvironmentPreservesCollectionControl ==
  ASSUME RetentionLifecycleInvariant, PreparedLifecycleInvariant', MetadataEnvironment
  PROVE CollectionControlInvariant'
  BY SMT, CompositionDomain, PublicationDomain,
      MetadataEnvironmentKeepsCollectionInputs, MetadataEnvironmentKeepsCollectionBarriers
  DEF RetentionLifecycleInvariant, CollectionControlInvariant, NoVisiblePendingWork,
      PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant, Invariant, MarkerCoversEntries,
      MetadataEnvironment, collectionControl, CachedEntriesEmpty, RetentionTypes,
      Publication!Invariant, Publication!TypeOK, Publication!GenerationIds, Publication!Parameters

THEOREM LifecycleOperationPreservesCollectionControl ==
  ASSUME NEW admitted \in BOOLEAN, RetentionLifecycleInvariant, LifecycleOperation(admitted)
  PROVE CollectionControlInvariant'
  BY SMT DEF RetentionLifecycleInvariant, CollectionControlInvariant, LifecycleOperation, collectionControl

THEOREM CollectionAcquisitionPreservesControl ==
  ASSUME RetentionLifecycleInvariant, AcquireCollection
  PROVE CollectionControlInvariant'
  BY SMT, CorrectCollection
  DEF RetentionLifecycleInvariant, CollectionControlInvariant, AcquireCollection,
      NoVisiblePendingWork, SelectionGeneration, preparedLifecycleVars, activationLifecycleVars,
      lifecycleVars, publicationVars, vars, selectionVars, hookVars, preparationHistoryVars, CachedEntriesEmpty

THEOREM CurrentAdmissionPreservesCollectionControl ==
  ASSUME RetentionLifecycleInvariant, SealCollectionCurrent
  PROVE CollectionControlInvariant'
  BY SMT DEF RetentionLifecycleInvariant, CollectionControlInvariant, SealCollectionCurrent,
      WritebackLifecycleCurrent, JournalHomeBarrier, Selection!SealCurrent, HookCurrentWriteback,
      Hooks!WritebackCurrent, Hooks!process, NoVisiblePendingWork,
      publicationVars, hookVars, preparationHistoryVars, CachedEntriesEmpty

THEOREM GenerationAdmissionPreservesCollectionControl ==
  ASSUME RetentionLifecycleInvariant, SealCollectionInventory
  PROVE CollectionControlInvariant'
  BY SMT DEF RetentionLifecycleInvariant, CollectionControlInvariant, SealCollectionInventory,
      Publication!WritebackGenerations, publicationVars, nonGenerationVars,
      vars, selectionVars, hookVars, preparationHistoryVars, NoVisiblePendingWork, CachedEntriesEmpty

THEOREM FloorObservationPreservesCollectionControl ==
  ASSUME NEW floor \in GenerationIds, RetentionLifecycleInvariant, ObserveCollectionFloor(floor)
  PROVE CollectionControlInvariant'
  BY SMT DEF RetentionLifecycleInvariant, CollectionControlInvariant, ObserveCollectionFloor,
      Publication!ObserveAllocationFloor, nonGenerationVars, vars, selectionVars, hookVars,
      preparationHistoryVars, NoVisiblePendingWork, CachedEntriesEmpty

THEOREM FloorBarrierPreservesCollectionControl ==
  ASSUME RetentionLifecycleInvariant, SealCollectionFloor
  PROVE CollectionControlInvariant'
  BY SMT, CorrectCollection, CompositionDomain
  DEF RetentionLifecycleInvariant, RetentionTypes, CollectionControlInvariant, SealCollectionFloor,
      Publication!SealAllocationFloor, Publication!GenerationIds, nonGenerationVars, vars,
      selectionVars, hookVars, preparationHistoryVars, NoVisiblePendingWork, CachedEntriesEmpty

THEOREM GenerationRemovalPreservesCollectionControl ==
  ASSUME NEW generation \in GenerationIds, RetentionLifecycleInvariant, RemoveCollectedGeneration(generation)
  PROVE CollectionControlInvariant'
  BY SMT DEF RetentionLifecycleInvariant, CollectionControlInvariant, RemoveCollectedGeneration,
      Publication!PruneGeneration, collectionControl, nonGenerationVars, vars, selectionVars,
      hookVars, preparationHistoryVars, NoVisiblePendingWork, CachedEntriesEmpty

THEOREM GenerationPruneCompletionPreservesCollectionControl ==
  ASSUME RetentionLifecycleInvariant, FinishGenerationPruning
  PROVE CollectionControlInvariant'
  BY SMT DEF RetentionLifecycleInvariant, CollectionControlInvariant, FinishGenerationPruning,
      preparedLifecycleVars, activationLifecycleVars, lifecycleVars, publicationVars,
      vars, selectionVars, hookVars, preparationHistoryVars, NoVisiblePendingWork, CachedEntriesEmpty

THEOREM GenerationPruneBarrierPreservesCollectionControl ==
  ASSUME RetentionLifecycleInvariant, SealPrunedGenerationNames
  PROVE CollectionControlInvariant'
  BY SMT, CorrectCollection
  DEF RetentionLifecycleInvariant, CollectionControlInvariant, SealPrunedGenerationNames,
      Publication!WritebackGenerations, nonGenerationVars, vars, selectionVars,
      hookVars, preparationHistoryVars, NoVisiblePendingWork, CachedEntriesEmpty

THEOREM InterruptedCollectionLosesOnlyCoordinatorState ==
  ASSUME RetentionLifecycleInvariant, ProcessInterruption \/ StorageInterruption
  PROVE CollectionControlInvariant'
  BY SMT DEF CollectionControlInvariant, ProcessInterruption, StorageInterruption,
      PreparedProcessCrash, PreparedStorageCrash, LifecycleProcessCrash, LifecycleStorageCrash,
      LifecycleProcessDeath, LifecyclePowerLoss, Publication!ProcessDeath, Publication!PowerLoss,
      Hooks!ProcessDeath, Hooks!PowerLoss, hookVars

THEOREM OtherRetentionStepsPreserveCollectionControl ==
  ASSUME RetentionLifecycleInvariant,
         AcquireLifecycle \/ (\E payload \in Payloads : AdmitPayload(payload) \/ RemoveCollectedPayload(payload))
         \/ FinishCollection \/ PayloadWriteback
  PROVE CollectionControlInvariant'
  BY SMT DEF RetentionLifecycleInvariant, CollectionControlInvariant, AcquireLifecycle, AdmitPayload,
      RemoveCollectedPayload, FinishCollection, PayloadWriteback, collectionControl,
      preparedLifecycleVars, activationLifecycleVars, lifecycleVars, publicationVars,
      vars, selectionVars, hookVars, preparationHistoryVars, NoVisiblePendingWork, CachedEntriesEmpty

THEOREM RetentionPreservesCollectionControl ==
  ASSUME NEW admitted \in BOOLEAN, RetentionLifecycleInvariant, RetentionLifecycleNext(admitted)
  PROVE CollectionControlInvariant'
  BY SMT, RetentionPreservesHigherProtocol, MetadataEnvironmentPreservesCollectionControl,
      LifecycleOperationPreservesCollectionControl, CollectionAcquisitionPreservesControl,
      CurrentAdmissionPreservesCollectionControl, GenerationAdmissionPreservesCollectionControl,
      FloorObservationPreservesCollectionControl, FloorBarrierPreservesCollectionControl,
      GenerationRemovalPreservesCollectionControl, GenerationPruneCompletionPreservesCollectionControl,
      GenerationPruneBarrierPreservesCollectionControl, InterruptedCollectionLosesOnlyCoordinatorState,
      OtherRetentionStepsPreserveCollectionControl
  DEF RetentionLifecycleNext

=============================================================================
