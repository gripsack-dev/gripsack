---- MODULE GenerationRetentionProofs ----
EXTENDS GenerationPublicationProofs

THEOREM ObservedFloorPreservesGenerationInvariant ==
  ASSUME NEW floor \in GenerationIds, Invariant, ObserveAllocationFloor(floor)
  PROVE Invariant'
  BY SMT DEF Invariant, TypeOK, GenerationIds, StorageInvariant, ControlInvariant,
      ReturnedPublicationIsStable, ObserveAllocationFloor

THEOREM SealedFloorPreservesGenerationInvariant ==
  Invariant /\ SealAllocationFloor => Invariant'
  BY SMT DEF Invariant, TypeOK, GenerationIds, StorageInvariant, ControlInvariant,
      ReturnedPublicationIsStable, SealAllocationFloor

THEOREM PruneAfterCounterPreservesGenerationInvariant ==
  ASSUME NEW generation \in GenerationIds, Invariant,
         generation <= waterStable, PruneGeneration(generation)
  PROVE Invariant'
  BY SMT DEF Invariant, TypeOK, StorageInvariant, ControlInvariant,
      ReturnedPublicationIsStable, PruneGeneration

THEOREM ObservedInventoryAndCounterCoverEveryPublishedId ==
  ASSUME NEW floor \in GenerationIds, Invariant, waterVisible <= floor,
         \A generation \in visible : generation <= floor
  PROVE \A generation \in everPublished : generation <= floor
  BY SMT DEF Invariant, TypeOK, GenerationIds, StorageInvariant

THEOREM PruningDoesNotReassignHistory ==
  ASSUME NEW generation \in GenerationIds, PruneGeneration(generation)
  PROVE UNCHANGED <<everPublished, ready, waterVisible, waterStable>>
  BY DEF PruneGeneration

=============================================================================
