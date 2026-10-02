---- MODULE LifecycleRetentionProofs ----
EXTENDS LifecycleRetentionControlProofs

THEOREM ReferencedRootsAreTyped ==
  ASSUME NEW generations \in SUBSET GenerationIds
  PROVE Roots(generations) \subseteq Payloads
  BY SMT, RetentionDomain DEF Roots

THEOREM RetentionInitialSafety == RetentionLifecycleInit => RetentionLifecycleInvariant
<1>1. RetentionLifecycleInit => PreparedLifecycleInvariant
  BY SMT, PreparedLifecycleInitialSafety DEF RetentionLifecycleInit
<1>2. RetentionLifecycleInit => RetentionTypes /\ CollectionControlInvariant /\ NoProtectedRootCollection
  BY SMT, RetentionDomain, CompositionDomain, PublicationDomain
  DEF RetentionLifecycleInit, RetentionTypes, CollectionControlInvariant, NoProtectedRootCollection,
      Coordinators, GcPhases, Selection!Selections, Publication!Parameters, Publication!GenerationIds,
      PreparedLifecycleInit, ActivationLifecycleInit, LifecycleInit, Publication!Init, Hooks!Init
<1>3. QED BY ONLY SMT, <1>1, <1>2 DEF RetentionLifecycleInvariant

THEOREM RetentionAcquisitionPreservesTypes ==
  ASSUME RetentionLifecycleInvariant, AcquireCollection
  PROVE RetentionTypes'
  BY SMT, ReferencedRootsAreTyped, CompositionDomain, PublicationDomain
  DEF RetentionLifecycleInvariant, RetentionTypes, AcquireCollection, Coordinators, GcPhases,
      PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant, Publication!Invariant,
      Publication!TypeOK, Publication!Parameters, Publication!GenerationIds, Invariant, TypeOK, Selection!TypeOK,
      payloadVars

THEOREM RetentionControlUpdatesPreserveTypes ==
  ASSUME RetentionLifecycleInvariant,
         AcquireLifecycle \/ SealCollectionCurrent \/ SealCollectionInventory
         \/ (\E floor \in GenerationIds : ObserveCollectionFloor(floor)) \/ SealCollectionFloor
         \/ (\E generation \in GenerationIds : RemoveCollectedGeneration(generation))
         \/ FinishGenerationPruning \/ SealPrunedGenerationNames \/ FinishCollection
  PROVE RetentionTypes'
  BY SMT, CompositionDomain, PublicationDomain
  DEF RetentionLifecycleInvariant, RetentionTypes, Coordinators, GcPhases,
      AcquireLifecycle, SealCollectionCurrent, SealCollectionInventory, ObserveCollectionFloor,
      SealCollectionFloor, RemoveCollectedGeneration, FinishGenerationPruning, SealPrunedGenerationNames,
      FinishCollection, collectionControl, payloadVars, Selection!Selections,
      Publication!Parameters, Publication!GenerationIds

THEOREM RetentionPayloadUpdatesPreserveTypes ==
  ASSUME RetentionLifecycleInvariant,
         (\E payload \in Payloads : AdmitPayload(payload) \/ RemoveCollectedPayload(payload)) \/ PayloadWriteback
  PROVE RetentionTypes'
  BY SMT DEF RetentionLifecycleInvariant, RetentionTypes, AdmitPayload, RemoveCollectedPayload,
      PayloadWriteback, collectionControl, payloadVars

THEOREM RetentionInterruptionsPreserveTypes ==
  ASSUME RetentionLifecycleInvariant, ProcessInterruption \/ StorageInterruption
  PROVE RetentionTypes'
  BY SMT, CompositionDomain, PublicationDomain
  DEF RetentionLifecycleInvariant, RetentionTypes, ProcessInterruption, StorageInterruption,
      Coordinators, GcPhases, Selection!Selections, Publication!Parameters, Publication!GenerationIds, payloadVars

THEOREM RetentionPreservesTypes ==
  ASSUME NEW admitted \in BOOLEAN, RetentionLifecycleInvariant, RetentionLifecycleNext(admitted)
  PROVE RetentionTypes'
  BY SMT, RetentionAcquisitionPreservesTypes, RetentionControlUpdatesPreserveTypes,
      RetentionPayloadUpdatesPreserveTypes, RetentionInterruptionsPreserveTypes
  DEF RetentionLifecycleNext, LifecycleOperation, MetadataEnvironment, collectionControl, payloadVars,
      RetentionLifecycleInvariant, RetentionTypes

THEOREM RetentionPreservesProtectedRoots ==
  ASSUME NEW admitted \in BOOLEAN, RetentionLifecycleInvariant, RetentionLifecycleNext(admitted)
  PROVE NoProtectedRootCollection'
  BY SMT, CollectionPreservesExistingProtectedPayloads
  DEF RetentionLifecycleInvariant, NoProtectedRootCollection, RetentionLifecycleNext,
      AcquireLifecycle, LifecycleOperation, AdmitPayload, AcquireCollection, SealCollectionCurrent,
      SealCollectionInventory, ObserveCollectionFloor, SealCollectionFloor, RemoveCollectedGeneration,
      FinishGenerationPruning, SealPrunedGenerationNames, FinishCollection,
      MetadataEnvironment, PayloadWriteback, ProcessInterruption, StorageInterruption, payloadVars

THEOREM RetentionInduction ==
  ASSUME NEW admitted \in BOOLEAN, RetentionLifecycleInvariant, RetentionLifecycleNext(admitted)
  PROVE RetentionLifecycleInvariant'
  BY RetentionPreservesHigherProtocol, RetentionPreservesCollectionControl,
      RetentionPreservesTypes, RetentionPreservesProtectedRoots
  DEF RetentionLifecycleInvariant

THEOREM RetentionPredicateFrame ==
  ASSUME RetentionLifecycleInvariant, UNCHANGED retentionLifecycleVars
  PROVE RetentionLifecycleInvariant'
  BY SMT, PreparedLifecyclePredicateFrame
  DEF RetentionLifecycleInvariant, RetentionTypes, CollectionControlInvariant, NoProtectedRootCollection,
      NoVisiblePendingWork, CachedEntriesEmpty, retentionLifecycleVars, collectionControl, payloadVars,
      preparedLifecycleVars, activationLifecycleVars, lifecycleVars, publicationVars,
      vars, selectionVars, hookVars, preparationHistoryVars

THEOREM GeneralLifecycleRetentionSafety == RetentionLifecycleSpec => []RetentionLifecycleInvariant
<1>1. RetentionLifecycleInit => RetentionLifecycleInvariant BY RetentionInitialSafety
<1>2. RetentionLifecycleInvariant /\ [RetentionLifecycleNext(TRUE)]_retentionLifecycleVars => RetentionLifecycleInvariant'
  BY SMT, RetentionInduction, RetentionPredicateFrame
<1>3. QED BY PTL, <1>1, <1>2 DEF RetentionLifecycleSpec

THEOREM PrunedGenerationIdsRemainCovered ==
  RetentionLifecycleInvariant => AllocationHistoryCovered
  BY DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant,
      LifecycleInvariant, Publication!Invariant, Publication!StorageInvariant, AllocationHistoryCovered

THEOREM CollectionCannotDeleteProtectedExistingPayload ==
  ASSUME NEW payload \in Payloads, RetentionLifecycleInvariant, RemoveCollectedPayload(payload)
  PROVE payloadsC \intersect ProtectedRoots \subseteq payloadsC'
  BY CollectionPreservesExistingProtectedPayloads DEF RetentionLifecycleInvariant

=============================================================================
