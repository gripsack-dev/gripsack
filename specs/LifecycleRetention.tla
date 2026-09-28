---- MODULE LifecycleRetention ----
EXTENDS PreparedActivationLifecycle

CONSTANTS Payloads, InitialPayloads, References, PriorRoots, GcMutant
ASSUME RetentionDomain ==
    /\ IsFiniteSet(Payloads) /\ InitialPayloads \subseteq Payloads
    /\ References \in [GenerationIds -> SUBSET Payloads]
    /\ PriorRoots \in [Objects -> SUBSET Payloads]
    /\ GcMutant \in {"none", "ignore_pending", "missing_prune_barrier", "missing_floor"}

Roots(generations) == UNION {References[generation] : generation \in generations}
SelectionGeneration(selection) == IF selection = NoSelection THEN {} ELSE {selection[2]}
JournalRoots == UNION {PriorRoots[cells[destination].original] : destination \in
    {rootDestination \in Destinations : cells[rootDestination].cached.entry.present \/ cells[rootDestination].durable.entry.present}}
ActivationRoots == UNION {References[plans[transaction]] : transaction \in
    {pendingTransaction \in Transactions : pendingTransaction = pendingC \/ (pendingTransaction = pendingD /\ pendingTransaction \notin archiveD)}}
ProtectedRoots == Roots(stable) \union JournalRoots \union ActivationRoots
NoVisiblePendingWork == ~markerVisible.present /\ CachedEntriesEmpty /\ pendingC = ActivationNone

Coordinators == {"none", "lifecycle", "gc"}
GcPhases == {"idle", "admit-current", "admit-names", "floor-write", "floor-seal", "prune", "prune-seal", "collect"}
VARIABLES coordinator, gcPhase, gcInventory, gcSelected, gcRoots, gcCurrent, gcFloor,
          payloadsC, payloadsD, unsafeCollection
collectionControl == <<coordinator, gcPhase, gcInventory, gcSelected, gcRoots, gcCurrent, gcFloor>>
payloadVars == <<payloadsC, payloadsD, unsafeCollection>>
nonGenerationVars == <<vars, hookVars, preparationVars, preparationHistoryVars>>
retentionLifecycleVars == <<preparedLifecycleVars, collectionControl, payloadVars>>

RetentionLifecycleInit ==
    /\ PreparedLifecycleInit
    /\ coordinator = "none" /\ gcPhase = "idle" /\ gcInventory = {} /\ gcSelected = {}
    /\ gcRoots = {} /\ gcCurrent = NoSelection /\ gcFloor = 0
    /\ payloadsC = InitialPayloads /\ payloadsD = InitialPayloads /\ unsafeCollection = FALSE

AcquireLifecycle ==
    /\ coordinator = "none" /\ coordinator' = "lifecycle"
    /\ UNCHANGED <<preparedLifecycleVars, gcPhase, gcInventory, gcSelected, gcRoots, gcCurrent, gcFloor, payloadVars>>
LifecycleOperation(admitted) ==
    /\ coordinator = "lifecycle"
    /\ (OrdinaryPreparedLifecycleStep(admitted)
        \/ (admitted /\ ((\E transaction \in Transactions : BeginPreparation(transaction))
              \/ PreparationIO \/ (\E transaction \in Transactions, generation \in GenerationIds :
                    FinishPreparation(transaction, generation)))))
    /\ UNCHANGED <<collectionControl, payloadVars>>

\* This input is an already returned store/prior publication, not raw buffered
\* bytes. The object/namespace protocols provide its admission contract; no
\* payload existence is inferred from a manifest reference alone.
AdmitPayload(payload) ==
    /\ coordinator = "lifecycle" /\ payloadsC' = payloadsC \union {payload}
    /\ payloadsD' = payloadsD \union {payload}
    /\ UNCHANGED <<preparedLifecycleVars, collectionControl, unsafeCollection>>

AcquireCollection ==
    /\ coordinator = "none"
    /\ GcMutant = "ignore_pending" \/ NoVisiblePendingWork
    /\ IF currentVisible = NoSelection THEN TRUE ELSE currentVisible[2] \in visible
    /\ coordinator' = "gc" /\ gcPhase' = "admit-current"
    /\ gcInventory' = visible /\ gcCurrent' = currentVisible
    /\ gcSelected' \in SUBSET (visible \ SelectionGeneration(currentVisible))
    /\ gcRoots' = Roots(visible \ gcSelected') /\ gcFloor' = 0
    /\ UNCHANGED <<preparedLifecycleVars, payloadVars>>
SealCollectionCurrent ==
    /\ coordinator = "gc" /\ gcPhase = "admit-current"
    /\ WritebackLifecycleCurrent /\ gcPhase' = "admit-names"
    /\ UNCHANGED <<preparationVars, preparationHistoryVars, coordinator,
                    gcInventory, gcSelected, gcRoots, gcCurrent, gcFloor, payloadVars>>
SealCollectionInventory ==
    /\ coordinator = "gc" /\ gcPhase = "admit-names"
    /\ IF visible = {} THEN UNCHANGED publicationVars
       ELSE Publication!WritebackGenerations /\ stable' = visible
    /\ gcPhase' = "floor-write"
    /\ UNCHANGED <<nonGenerationVars, admittedGenerations, coordinator,
                    gcInventory, gcSelected, gcRoots, gcCurrent, gcFloor, payloadVars>>
ObserveCollectionFloor(floor) ==
    /\ coordinator = "gc" /\ gcPhase = "floor-write"
    /\ Publication!ObserveAllocationFloor(floor)
    /\ gcFloor' = floor /\ gcPhase' = "floor-seal"
    /\ UNCHANGED <<nonGenerationVars, admittedGenerations, coordinator,
                    gcInventory, gcSelected, gcRoots, gcCurrent, payloadVars>>
SealCollectionFloor ==
    /\ coordinator = "gc" /\ gcPhase = "floor-seal"
    /\ IF GcMutant = "missing_floor" THEN UNCHANGED publicationVars ELSE Publication!SealAllocationFloor
    /\ gcPhase' = "prune"
    /\ UNCHANGED <<nonGenerationVars, admittedGenerations, coordinator,
                    gcInventory, gcSelected, gcRoots, gcCurrent, gcFloor, payloadVars>>
RemoveCollectedGeneration(generation) ==
    /\ coordinator = "gc" /\ gcPhase = "prune" /\ generation \in gcSelected
    /\ Publication!PruneGeneration(generation)
    /\ admittedGenerations' = admittedGenerations \ {generation}
    /\ UNCHANGED <<nonGenerationVars, collectionControl, payloadVars>>
FinishGenerationPruning ==
    /\ coordinator = "gc" /\ gcPhase = "prune" /\ gcSelected \intersect visible = {}
    /\ gcPhase' = "prune-seal"
    /\ UNCHANGED <<preparedLifecycleVars, coordinator, gcInventory, gcSelected, gcRoots, gcCurrent, gcFloor, payloadVars>>
SealPrunedGenerationNames ==
    /\ coordinator = "gc" /\ gcPhase = "prune-seal"
    /\ IF GcMutant = "missing_prune_barrier" THEN UNCHANGED publicationVars
       ELSE Publication!WritebackGenerations /\ stable' = visible
    /\ gcPhase' = "collect"
    /\ UNCHANGED <<nonGenerationVars, admittedGenerations, coordinator,
                    gcInventory, gcSelected, gcRoots, gcCurrent, gcFloor, payloadVars>>
RemoveCollectedPayload(payload) ==
    /\ coordinator = "gc" /\ gcPhase = "collect" /\ payload \in payloadsC \ gcRoots
    /\ payloadsC' = payloadsC \ {payload}
    /\ unsafeCollection' = (unsafeCollection \/ payload \in ProtectedRoots)
    /\ UNCHANGED <<preparedLifecycleVars, collectionControl, payloadsD>>
FinishCollection ==
    /\ coordinator = "gc" /\ gcPhase = "collect"
    /\ coordinator' = "none" /\ gcPhase' = "idle" /\ gcInventory' = {} /\ gcSelected' = {}
    /\ gcRoots' = {} /\ gcCurrent' = NoSelection /\ gcFloor' = 0
    /\ UNCHANGED <<preparedLifecycleVars, payloadVars>>

\* Kernel writeback and external destination edits remain possible while either
\* cooperating command owns the session. Private metadata has no outside writer.
MetadataEnvironment ==
    /\ ((PublicationWithHooks /\ (Publication!WritebackWater \/ Publication!WritebackGenerations))
        \/ StationaryJournalStep(FALSE) \/ WritebackLifecycleCurrent \/ ActivationStorageStep)
    /\ UNCHANGED <<preparationVars, preparationHistoryVars, collectionControl, payloadVars>>
PayloadWriteback ==
    /\ payloadsD' \in {next \in SUBSET payloadsD : payloadsC \subseteq next}
    /\ UNCHANGED <<preparedLifecycleVars, collectionControl, payloadsC, unsafeCollection>>
ProcessInterruption ==
    /\ PreparedProcessCrash
    /\ coordinator' = "none" /\ gcPhase' = "idle" /\ gcInventory' = {} /\ gcSelected' = {}
    /\ gcRoots' = {} /\ gcCurrent' = NoSelection /\ gcFloor' = 0
    /\ UNCHANGED payloadVars
StorageInterruption ==
    /\ PreparedStorageCrash
    /\ payloadsD' \in {next \in SUBSET payloadsD : payloadsC \subseteq next} /\ payloadsC' = payloadsD'
    /\ coordinator' = "none" /\ gcPhase' = "idle" /\ gcInventory' = {} /\ gcSelected' = {}
    /\ gcRoots' = {} /\ gcCurrent' = NoSelection /\ gcFloor' = 0
    /\ UNCHANGED unsafeCollection

RetentionLifecycleNext(admitted) == AcquireLifecycle \/ LifecycleOperation(admitted)
    \/ (admitted /\ ((\E payload \in Payloads : AdmitPayload(payload)) \/ AcquireCollection
          \/ SealCollectionCurrent \/ SealCollectionInventory
          \/ (\E floor \in GenerationIds : ObserveCollectionFloor(floor)) \/ SealCollectionFloor
          \/ (\E generation \in GenerationIds : RemoveCollectedGeneration(generation))
          \/ FinishGenerationPruning \/ SealPrunedGenerationNames
          \/ (\E payload \in Payloads : RemoveCollectedPayload(payload)) \/ FinishCollection))
    \/ MetadataEnvironment \/ PayloadWriteback \/ ProcessInterruption \/ StorageInterruption
RetentionLifecycleSpec == RetentionLifecycleInit /\ [][RetentionLifecycleNext(TRUE)]_retentionLifecycleVars

RetentionTypes ==
    /\ coordinator \in Coordinators /\ gcPhase \in GcPhases
    /\ gcInventory \subseteq GenerationIds /\ gcSelected \subseteq gcInventory /\ gcRoots \subseteq Payloads
    /\ gcCurrent \in Selection!Selections /\ gcFloor \in GenerationIds
    /\ payloadsC \subseteq payloadsD /\ payloadsD \subseteq Payloads /\ unsafeCollection \in BOOLEAN
CollectionControlInvariant ==
    /\ (coordinator = "gc" <=> gcPhase # "idle")
    /\ (coordinator # "lifecycle" => publicationPhase = "idle" /\ activationPhase = "idle" /\ preparationOwner = ActivationNone)
    /\ (coordinator = "gc" =>
          /\ NoVisiblePendingWork /\ currentVisible = gcCurrent
          /\ gcSelected \intersect SelectionGeneration(gcCurrent) = {}
          /\ visible \subseteq gcInventory /\ gcInventory \ visible \subseteq gcSelected
          /\ gcRoots = Roots(gcInventory \ gcSelected))
    /\ (gcPhase \in {"admit-current", "admit-names", "floor-write", "floor-seal"} => visible = gcInventory)
    /\ (gcPhase # "admit-current" /\ coordinator = "gc" => currentStable = gcCurrent)
    /\ (gcPhase = "floor-seal" => waterVisible = gcFloor /\ (\A generation \in gcInventory : generation <= gcFloor))
    /\ (gcPhase \in {"prune", "prune-seal", "collect"} =>
          /\ gcFloor <= waterStable /\ (\A generation \in gcInventory : generation <= gcFloor))
    /\ (gcPhase \in {"prune-seal", "collect"} => visible = gcInventory \ gcSelected)
    /\ (gcPhase = "collect" => stable = visible)
NoProtectedRootCollection == ~unsafeCollection
AllocationHistoryCovered == \A generation \in everPublished :
    generation <= waterStable \/ generation \in visible \intersect stable
RetentionLifecycleInvariant == PreparedLifecycleInvariant /\ RetentionTypes /\ CollectionControlInvariant
    /\ NoProtectedRootCollection

=============================================================================
