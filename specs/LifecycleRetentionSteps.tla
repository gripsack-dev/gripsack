---- MODULE LifecycleRetentionSteps ----
EXTENDS LifecycleRetentionBridgeProofs

GenerationCollectionAction == SealCollectionInventory
    \/ (\E floor \in GenerationIds : ObserveCollectionFloor(floor)) \/ SealCollectionFloor
    \/ (\E generation \in GenerationIds : RemoveCollectedGeneration(generation)) \/ SealPrunedGenerationNames

THEOREM CollectionFramesNonGenerationState ==
  GenerationCollectionAction => UNCHANGED nonGenerationVars
  BY SMT DEF GenerationCollectionAction, SealCollectionInventory, ObserveCollectionFloor,
      SealCollectionFloor, RemoveCollectedGeneration, SealPrunedGenerationNames

THEOREM CollectionPreservesGenerationProtocol ==
  ASSUME RetentionLifecycleInvariant, GenerationCollectionAction
  PROVE Publication!Invariant'
<1>1. Publication!Invariant
  BY DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant
<1>2. CASE SealCollectionInventory \/ SealPrunedGenerationNames
  BY SMT, <1>1, <1>2, CorrectCollection, PublicationComponentInduction, PublicationStutterPreservesInvariant
  DEF SealCollectionInventory, SealPrunedGenerationNames, Publication!Next
<1>3. CASE \E floor \in GenerationIds : ObserveCollectionFloor(floor)
  BY SMT, <1>1, <1>3, ObserveFloorComponentSafety DEF ObserveCollectionFloor
<1>4. CASE SealCollectionFloor
  BY SMT, <1>1, <1>4, CorrectCollection, SealFloorComponentSafety DEF SealCollectionFloor
<1>5. CASE \E generation \in GenerationIds : RemoveCollectedGeneration(generation)
  <2>1. PICK generation \in GenerationIds : RemoveCollectedGeneration(generation) BY <1>5
  <2>2. generation <= waterStable
    BY SMT, <1>1, <2>1, CompositionDomain
    DEF RetentionLifecycleInvariant, RetentionTypes, CollectionControlInvariant,
        RemoveCollectedGeneration, Publication!Invariant, Publication!TypeOK, Publication!GenerationIds
  <2>3. QED BY SMT, <1>1, <2>1, <2>2, PruneGenerationComponentSafety DEF RemoveCollectedGeneration
<1>6. QED BY ONLY SMT, GenerationCollectionAction, <1>2, <1>3, <1>4, <1>5 DEF GenerationCollectionAction

THEOREM CollectionPreservesGenerationAdmission ==
  ASSUME RetentionLifecycleInvariant, GenerationCollectionAction
  PROVE admittedGenerations' \subseteq (stable' \intersect visible')
  BY SMT, CorrectCollection
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant,
      GenerationCollectionAction, SealCollectionInventory, ObserveCollectionFloor, SealCollectionFloor,
      RemoveCollectedGeneration, SealPrunedGenerationNames, Publication!ObserveAllocationFloor,
      Publication!SealAllocationFloor, Publication!PruneGeneration, Publication!WritebackGenerations, publicationVars

THEOREM CollectionPreservesCurrentGeneration ==
  ASSUME RetentionLifecycleInvariant, GenerationCollectionAction
  PROVE CurrentNamesDurableGeneration'
  BY SMT, CorrectCollection
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant,
      CollectionControlInvariant, CurrentNamesDurableGeneration, SelectionGeneration,
      GenerationCollectionAction, SealCollectionInventory, ObserveCollectionFloor, SealCollectionFloor,
      RemoveCollectedGeneration, SealPrunedGenerationNames, Publication!ObserveAllocationFloor,
      Publication!SealAllocationFloor, Publication!PruneGeneration, Publication!WritebackGenerations,
      nonGenerationVars, vars, selectionVars,
      publicationVars

THEOREM CollectionPreservesHigherProtocol ==
  ASSUME RetentionLifecycleInvariant, GenerationCollectionAction
  PROVE PreparedLifecycleInvariant'
  BY SMT, CollectionFramesNonGenerationState, CollectionPreservesGenerationProtocol,
      CollectionPreservesGenerationAdmission, CollectionPreservesCurrentGeneration, NonGenerationStateFrame
  DEF RetentionLifecycleInvariant

THEOREM MetadataEnvironmentProjectsPreparedLifecycle ==
  MetadataEnvironment => PreparedLifecycleNext(FALSE)
  BY SMT DEF MetadataEnvironment, PreparedLifecycleNext, OrdinaryPreparedLifecycleStep

THEOREM RetentionStepsProjectHigherProtocol ==
  ASSUME NEW admitted \in BOOLEAN, RetentionLifecycleNext(admitted)
  PROVE GenerationCollectionAction \/ UNCHANGED preparedLifecycleVars \/
        PreparedLifecycleNext(admitted) \/ PreparedLifecycleNext(FALSE)
  BY SMT, MetadataEnvironmentProjectsPreparedLifecycle
  DEF RetentionLifecycleNext, LifecycleOperation, PreparedLifecycleNext, GenerationCollectionAction,
      AcquireLifecycle, AdmitPayload, AcquireCollection, SealCollectionCurrent, FinishGenerationPruning,
      RemoveCollectedPayload, FinishCollection, PayloadWriteback, ProcessInterruption, StorageInterruption,
      OrdinaryPreparedLifecycleStep

THEOREM RetentionPreservesHigherProtocol ==
  ASSUME NEW admitted \in BOOLEAN, RetentionLifecycleInvariant, RetentionLifecycleNext(admitted)
  PROVE PreparedLifecycleInvariant'
  BY SMT, RetentionStepsProjectHigherProtocol, CollectionPreservesHigherProtocol,
      PreparedLifecyclePredicateFrame, PreparedLifecycleInduction
  DEF RetentionLifecycleInvariant

=============================================================================
