---- MODULE LifecycleRetentionBridgeProofs ----
EXTENDS LifecycleRetention, PreparedActivationProofs

ASSUME CorrectCollection == GcMutant = "none"

THEOREM RetentionPrimitiveNamesAgree ==
    /\ (\A floor \in GenerationIds :
          Publication!ObserveAllocationFloor(floor) = PublicationProofs!ObserveAllocationFloor(floor))
    /\ Publication!SealAllocationFloor = PublicationProofs!SealAllocationFloor
    /\ (\A generation \in GenerationIds :
          Publication!PruneGeneration(generation) = PublicationProofs!PruneGeneration(generation))
  BY DEF Publication!ObserveAllocationFloor, PublicationProofs!ObserveAllocationFloor,
      Publication!SealAllocationFloor, PublicationProofs!SealAllocationFloor,
      Publication!PruneGeneration, PublicationProofs!PruneGeneration,
      Publication!GenerationIds, PublicationProofs!GenerationIds

THEOREM ObserveFloorComponentSafety ==
  ASSUME NEW floor \in GenerationIds, Publication!Invariant, Publication!ObserveAllocationFloor(floor)
  PROVE Publication!Invariant'
  BY SMT, CompositionDomain, PublicationStateNamesAgree, PublicationProofPremises,
      RetentionPrimitiveNamesAgree, PublicationProofs!ObservedFloorPreservesGenerationInvariant
  DEF Publication!GenerationIds, PublicationProofs!GenerationIds

THEOREM SealFloorComponentSafety ==
  Publication!Invariant /\ Publication!SealAllocationFloor => Publication!Invariant'
  BY SMT, PublicationStateNamesAgree, PublicationProofPremises,
      RetentionPrimitiveNamesAgree, PublicationProofs!SealedFloorPreservesGenerationInvariant

THEOREM PruneGenerationComponentSafety ==
  ASSUME NEW generation \in GenerationIds, Publication!Invariant,
         generation <= waterStable, Publication!PruneGeneration(generation)
  PROVE Publication!Invariant'
  BY SMT, CompositionDomain, PublicationStateNamesAgree, PublicationProofPremises,
      RetentionPrimitiveNamesAgree, PublicationProofs!PruneAfterCounterPreservesGenerationInvariant
  DEF Publication!GenerationIds, PublicationProofs!GenerationIds

THEOREM NonGenerationStateFrame ==
  ASSUME PreparedLifecycleInvariant, UNCHANGED nonGenerationVars,
         Publication!Invariant', admittedGenerations' \subseteq (stable' \intersect visible'),
         CurrentNamesDurableGeneration'
  PROVE PreparedLifecycleInvariant'
<1>1. Invariant' /\ Hooks!ActivationInductiveInvariant' /\ Preparation!Invariant'
  BY SMT, JournalStutterPreservesInvariant, HookStutterPreservesInvariant, PreparationComponentFrame
  DEF PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant, nonGenerationVars
<1>2. CouplingInvariant' /\ PreparationHistoryInvariant'
  BY SMT, CouplingStateFrame
  DEF PreparedLifecycleInvariant, ActivationLifecycleInvariant, CouplingInvariant,
      PreparationHistoryInvariant, PreparationTypes, PreparationOwnerBinding, EveryPlanHasDurablePreparation,
      nonGenerationVars, couplingVars, vars, selectionVars, control, hookVars,
      preparationVars, preparationHistoryVars
<1>3. QED BY ONLY SMT, <1>1, <1>2, Publication!Invariant',
    admittedGenerations' \subseteq (stable' \intersect visible'), CurrentNamesDurableGeneration'
  DEF PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant,
      CouplingInvariant, PreparationHistoryInvariant

THEOREM PreparedLifecyclePredicateFrame ==
  ASSUME PreparedLifecycleInvariant, UNCHANGED preparedLifecycleVars
  PROVE PreparedLifecycleInvariant'
  BY SMT, ActivationLifecycleStateFrame, PreparationComponentFrame
  DEF PreparedLifecycleInvariant, PreparationTypes, PreparationOwnerBinding, EveryPlanHasDurablePreparation,
      preparedLifecycleVars, preparationHistoryVars, preparationVars, activationLifecycleVars,
      lifecycleVars, vars, selectionVars, control, hookVars

THEOREM NoObservedJournalImpliesNoJournalRoots ==
  ASSUME PreparedLifecycleInvariant, NoVisiblePendingWork
  PROVE JournalRoots = {}
  BY SMT DEF PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant,
      Invariant, MarkerCoversEntries, NoVisiblePendingWork, CachedEntriesEmpty, StableEntriesEmpty, JournalRoots

THEOREM NoObservedActivationImpliesNoActivationRoots ==
  ASSUME PreparedLifecycleInvariant, NoVisiblePendingWork
  PROVE ActivationRoots = {}
  BY SMT, HookDomain
  DEF PreparedLifecycleInvariant, ActivationLifecycleInvariant, Hooks!ActivationInductiveInvariant,
      Hooks!HomeInvariant, Hooks!ActivationParameters, NoVisiblePendingWork, ActivationRoots

THEOREM CollectedPayloadsAreOutsideAllProtectedRoots ==
  ASSUME PreparedLifecycleInvariant, CollectionControlInvariant, coordinator = "gc", gcPhase = "collect"
  PROVE ProtectedRoots \subseteq gcRoots
  BY SMT, NoObservedJournalImpliesNoJournalRoots, NoObservedActivationImpliesNoActivationRoots
  DEF CollectionControlInvariant, ProtectedRoots

THEOREM CollectionPreservesExistingProtectedPayloads ==
  ASSUME NEW payload \in Payloads, PreparedLifecycleInvariant, CollectionControlInvariant,
         RemoveCollectedPayload(payload)
  PROVE payloadsC \intersect ProtectedRoots \subseteq payloadsC' /\
        (NoProtectedRootCollection => NoProtectedRootCollection')
  BY SMT, CollectedPayloadsAreOutsideAllProtectedRoots
  DEF RemoveCollectedPayload, NoProtectedRootCollection

=============================================================================
