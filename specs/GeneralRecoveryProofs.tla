---- MODULE GeneralRecoveryProofs ----
EXTENDS LifecycleRetentionProofs, RetainedDirectoryProofs

GeneralRecoveryContract ==
    /\ RecoveryEvidencePreserved /\ MutationHasDurableMarker /\ ExactCommitIdentity
    /\ CurrentNamesDurableGeneration /\ AllocationHistoryCovered /\ NoProtectedRootCollection
    /\ EveryPlanHasDurablePreparation
    /\ Hooks!OutcomeAfterReturn /\ Hooks!TerminalNoReplay /\ Hooks!EffectsBindFullSelection
    /\ Hooks!ArchiveBeforeClear /\ Hooks!NoSilentSkip

THEOREM CommitClassificationUsesExactIdentity == ExactCommitIdentity
  BY SMT, CorrectJournal DEF ExactCommitIdentity, Classification, Selection!Classify

THEOREM JoinedInvariantImpliesRecoveryContract == RetentionLifecycleInvariant => GeneralRecoveryContract
  BY SMT, JournalInvariantPreservesRecoveryEvidence, CommitClassificationUsesExactIdentity,
      PrunedGenerationIdsRemainCovered
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant,
      LifecycleInvariant, GeneralRecoveryContract, Hooks!ActivationInductiveInvariant,
      Hooks!HomeInvariant, Hooks!HistoryInvariant, Hooks!ArchiveInvariant, Hooks!ArchiveBeforeClear

THEOREM ArbitraryFiniteRepeatedRecoverySafety == RetentionLifecycleSpec => []GeneralRecoveryContract
  BY PTL, GeneralLifecycleRetentionSafety, JoinedInvariantImpliesRecoveryContract

THEOREM JoinedEntryCleanupRequiresStableRestoration ==
  ASSUME NEW destination \in Destinations, RetentionLifecycleInvariant, ~Committed, RemoveEntry(destination)
  PROVE ~UnsafeOwned(cells[destination], cells[destination].durable.live)
  BY SMT, EntryRemovalRequiresDurableRestoration
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant

THEOREM JoinedHookInvocationBindsDurableSelection ==
  ASSUME RetentionLifecycleInvariant, Hooks!Invoke
  PROVE currentVisible = <<active, plans[active]>> /\ currentStable = currentVisible
  BY SMT, InvocationUsesDurableFullSelection
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant

THEOREM JoinedPendingResurrectionIsSettled ==
  ASSUME RetentionLifecycleInvariant, pendingC = ActivationNone, pendingD # ActivationNone
  PROVE pendingD \in archiveD /\ Hooks!AllSettled(pendingD, stateD)
  BY SMT, ClearedPendingResurrectionHasNoUnsettledIntents
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant

THEOREM RejectedAdmissionCannotRemoveVisiblePayloads ==
  ASSUME RetentionTypes, RetentionLifecycleNext(FALSE)
  PROVE payloadsC \subseteq payloadsC'
  BY SMT DEF RetentionTypes, RetentionLifecycleNext, AcquireLifecycle, LifecycleOperation, MetadataEnvironment,
      PayloadWriteback, ProcessInterruption, StorageInterruption, payloadVars

\* Directory authority is an additional domain, not another leaf classifier.
\* Existing flat-lifecycle theorems above keep exactly their previous contract.
ASSUME GeneralHierarchyDestinations == DirectoryLeaves = Destinations
ASSUME GeneralHierarchyPayloadDomain ==
    /\ DirectoryGenerations = GenerationIds /\ DirectoryPayloads = Payloads
    /\ DirectoryGenerationRoots = References

GeneralHierarchyLeafSettled(destination) ==
    /\ ~cells[destination].cached.entry.present
    /\ ~cells[destination].durable.entry.present
    /\ (Committed \/ (~UnsafeOwned(cells[destination], cells[destination].cached.live)
                       /\ ~UnsafeOwned(cells[destination], cells[destination].durable.live)))

GeneralHierarchyLeafAbsent(destination) ==
    /\ cells[destination].cached.live.value = Absent
    /\ cells[destination].durable.live.value = Absent

GeneralHierarchyJournalAuthority ==
    \A destination \in Destinations :
        ((WriterStep(destination) \/ RestoreStep(destination))
         /\ (cells'[destination].cached.live # cells[destination].cached.live
             \/ cells'[destination].durable.live # cells[destination].durable.live)) =>
            (destination \in directoryLeaves /\ DirectoryBindingMatches(destination))

GeneralHierarchyDirectoryAuthority ==
    /\ \A destination \in Destinations :
          DirectoryReleaseLeaf(destination) => GeneralHierarchyLeafSettled(destination)
    /\ \A parent \in DirectoryParents :
          (DirectoryRemove(parent) \/ DirectoryClearIntent(parent)
           \/ (DirectoryRestoreLink(parent) /\ directoryStages[parent] # DirectoryOwned(parent))) =>
              (\A destination \in Destinations :
                  parent \in DirectoryAncestors[destination] => GeneralHierarchyLeafSettled(destination))
    /\ \A destination \in Destinations :
          DirectorySuspendLeaf(destination) => GeneralHierarchyLeafAbsent(destination)
    /\ \A parent \in DirectoryParents :
          (DirectoryRemove(parent) \/ DirectoryRestoreLink(parent) \/ DirectoryStageOwned(parent)) =>
              (\A destination \in Destinations :
                  parent \in DirectoryAncestors[destination] => GeneralHierarchyLeafAbsent(destination))
    /\ \A receipt \in RecoveryReceiptKeys :
          (DirectoryRetainReceipt(receipt) \/ RetainMigrationReceipt(receipt)) =>
              RecoveryReceiptRoots({receipt}) \subseteq payloadsC
    /\ \A receipt \in RecoveryReceiptKeys :
          ReleaseRecoveryReceipt(receipt) => RecoveryReceiptTarget[receipt] \notin visible \union stable

GeneralHierarchyReceiptRootsPresent == RecoveryReceiptRoots(retainedReceipts) \subseteq payloadsC
GeneralHierarchyCollectionAuthority ==
    /\ \A payload \in RecoveryReceiptRoots(retainedReceipts) : ~RemoveCollectedPayload(payload)
    /\ \A receipt \in retainedReceipts :
          \A generation \in RecoveryReceiptLineage[receipt] \ {RecoveryReceiptTarget[receipt]} :
              ~RemoveCollectedGeneration(generation)
GeneralHierarchyVars == <<retentionLifecycleVars, RetainedDirectoryVars>>
GeneralHierarchyInit == RetentionLifecycleInit /\ RetainedDirectoryInit
GeneralHierarchyNext ==
    \/ RetentionLifecycleNext(TRUE) /\ UNCHANGED RetainedDirectoryVars
       /\ GeneralHierarchyJournalAuthority /\ GeneralHierarchyCollectionAuthority
    \/ RetainedDirectoryNext /\ UNCHANGED retentionLifecycleVars /\ GeneralHierarchyDirectoryAuthority
GeneralHierarchySpec == GeneralHierarchyInit /\ [][GeneralHierarchyNext]_GeneralHierarchyVars
GeneralHierarchyRecoveryContract ==
    GeneralRecoveryContract /\ DirectoryEvidence /\ DirectorySafety /\ GeneralHierarchyReceiptRootsPresent

THEOREM GeneralHierarchyStepProjectsLifecycle ==
  [GeneralHierarchyNext]_GeneralHierarchyVars => [RetentionLifecycleNext(TRUE)]_retentionLifecycleVars
  BY SMT DEF GeneralHierarchyNext, GeneralHierarchyVars

THEOREM GeneralHierarchyStepProjectsDirectory ==
  [GeneralHierarchyNext]_GeneralHierarchyVars => [RetainedDirectoryNext]_RetainedDirectoryVars
  BY SMT DEF GeneralHierarchyNext, GeneralHierarchyVars

THEOREM GeneralHierarchyProjectsLifecycle == GeneralHierarchySpec => RetentionLifecycleSpec
  BY PTL, GeneralHierarchyStepProjectsLifecycle
  DEF GeneralHierarchySpec, GeneralHierarchyInit, RetentionLifecycleSpec

THEOREM GeneralHierarchyProjectsDirectory == GeneralHierarchySpec => RetainedDirectorySpec
  BY PTL, GeneralHierarchyStepProjectsDirectory
  DEF GeneralHierarchySpec, GeneralHierarchyInit, RetainedDirectorySpec


THEOREM JoinedLeafMutationRequiresExactAncestors ==
  ASSUME NEW destination \in Destinations, GeneralHierarchyNext,
         WriterStep(destination) \/ RestoreStep(destination),
         cells'[destination].cached.live # cells[destination].cached.live
           \/ cells'[destination].durable.live # cells[destination].durable.live
  PROVE destination \in directoryLeaves /\ DirectoryBindingMatches(destination)
  BY SMT DEF GeneralHierarchyNext, GeneralHierarchyJournalAuthority,
      retentionLifecycleVars, preparedLifecycleVars, activationLifecycleVars,
      lifecycleVars, vars

THEOREM JoinedParentCleanupRequiresStableChildren ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, GeneralHierarchyNext,
         DirectoryRemove(parent)
           \/ (DirectoryRestoreLink(parent) /\ directoryStages[parent] # DirectoryOwned(parent))
  PROVE /\ DirectoryChildrenSettled(parent)
        /\ \A destination \in Destinations :
              parent \in DirectoryAncestors[destination] => GeneralHierarchyLeafSettled(destination)
  BY SMT, DirectoryCleanupIsChildBeforeParent
  DEF GeneralHierarchyNext, GeneralHierarchyDirectoryAuthority, RetainedDirectoryVars, DirectoryVars,
      DirectoryRemove, DirectoryRestoreLink, DirectoryOwned, DirectoryPrior,
      DirectoryInvariant, DirectoryTypeOK

THEOREM JoinedLeafAuthorityOutlivesOrdinaryJournal ==
  ASSUME NEW destination \in Destinations, GeneralHierarchyNext, DirectoryReleaseLeaf(destination)
  PROVE GeneralHierarchyLeafSettled(destination) /\ DirectoryBindingMatches(destination)
  BY SMT, DirectoryLeafRecoveryBindsAncestors
  DEF GeneralHierarchyNext, GeneralHierarchyDirectoryAuthority, DirectoryReleaseLeaf,
      RetainedDirectoryVars, DirectoryVars

THEOREM GeneralHierarchyInitialReceiptRoots ==
  GeneralHierarchyInit => GeneralHierarchyReceiptRootsPresent
  BY SMT DEF GeneralHierarchyInit, RetainedDirectoryInit, GeneralHierarchyReceiptRootsPresent,
      RecoveryReceiptRoots

THEOREM GeneralHierarchyLifecyclePreservesReceiptRoots ==
  ASSUME GeneralHierarchyReceiptRootsPresent, RetentionLifecycleNext(TRUE),
         GeneralHierarchyCollectionAuthority, UNCHANGED RetainedDirectoryVars
  PROVE GeneralHierarchyReceiptRootsPresent'
  BY SMT
  DEF GeneralHierarchyReceiptRootsPresent, GeneralHierarchyCollectionAuthority,
      RetainedDirectoryVars, DirectoryStageVars, RetentionLifecycleNext, AcquireLifecycle,
      LifecycleOperation, AdmitPayload, AcquireCollection, SealCollectionCurrent,
      SealCollectionInventory, ObserveCollectionFloor, SealCollectionFloor,
      RemoveCollectedGeneration, FinishGenerationPruning, SealPrunedGenerationNames,
      RemoveCollectedPayload, FinishCollection, MetadataEnvironment, PayloadWriteback,
      ProcessInterruption, StorageInterruption, payloadVars

THEOREM GeneralHierarchyDirectoryPreservesReceiptRoots ==
  ASSUME GeneralHierarchyReceiptRootsPresent, RetainedDirectoryNext,
         GeneralHierarchyDirectoryAuthority, UNCHANGED retentionLifecycleVars
  PROVE GeneralHierarchyReceiptRootsPresent'
  BY SMT, DirectoryReceiptParameters
  DEF GeneralHierarchyReceiptRootsPresent, GeneralHierarchyDirectoryAuthority,
      RetainedDirectoryNext, DirectoryRetainReceipt, RetainMigrationReceipt, ReleaseRecoveryReceipt,
      DirectoryStageOwned, DirectorySealReverseIntent, DirectoryReverseUnlink,
      DirectoryReinstallStage, DirectoryDiscardStage, DirectorySubstituteStage,
      DirectoryAddForeignStageChild, DirectorySuspendLeaf, DirectoryStageVars, RecoveryReceiptRoots,
      DirectoryWriteEarlierRoot, DirectoryNormalizeRoot, retentionLifecycleVars, payloadVars

THEOREM GeneralHierarchyReceiptRootsInduction ==
  GeneralHierarchyReceiptRootsPresent /\ GeneralHierarchyNext => GeneralHierarchyReceiptRootsPresent'
  BY SMT, GeneralHierarchyLifecyclePreservesReceiptRoots, GeneralHierarchyDirectoryPreservesReceiptRoots
  DEF GeneralHierarchyNext

THEOREM GeneralHierarchyReceiptRootsSafety ==
  GeneralHierarchySpec => []GeneralHierarchyReceiptRootsPresent
<1>1. GeneralHierarchyInit => GeneralHierarchyReceiptRootsPresent
  BY GeneralHierarchyInitialReceiptRoots
<1>2. GeneralHierarchyReceiptRootsPresent /\ [GeneralHierarchyNext]_GeneralHierarchyVars
       => GeneralHierarchyReceiptRootsPresent'
  BY GeneralHierarchyReceiptRootsInduction
  DEF GeneralHierarchyReceiptRootsPresent, RecoveryReceiptRoots, GeneralHierarchyVars,
      retentionLifecycleVars, payloadVars, RetainedDirectoryVars, DirectoryStageVars
<1>3. QED
  BY PTL, <1>1, <1>2 DEF GeneralHierarchySpec

THEOREM ArbitraryFiniteHierarchyRecoverySafety ==
  GeneralHierarchySpec => []GeneralHierarchyRecoveryContract
  BY PTL, GeneralHierarchyProjectsLifecycle, GeneralHierarchyProjectsDirectory,
      ArbitraryFiniteRepeatedRecoverySafety, ArbitraryFiniteRetainedDirectorySafety,
      GeneralHierarchyReceiptRootsSafety
  DEF GeneralHierarchyRecoveryContract, RetainedDirectoryInvariant, DirectoryInvariant

THEOREM JoinedReceiptPredecessorRootsRemainPresent ==
  ASSUME GeneralHierarchyReceiptRootsPresent, RetainedDirectoryInvariant,
         NEW receipt \in retainedReceipts, NEW generation \in RecoveryReceiptLineage[receipt],
         NEW predecessor \in DirectoryGenerationPredecessors[generation]
  PROVE DirectoryGenerationRoots[predecessor] \subseteq payloadsC
  BY SMT, ReceiptLineageContainsPredecessorRoots
  DEF GeneralHierarchyReceiptRootsPresent, RetainedDirectoryInvariant

THEOREM JoinedReverseStagingRequiresStableChildren ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents,
         GeneralHierarchyNext, DirectoryStageOwned(parent)
  PROVE /\ DirectoryChildrenSettled(parent)
        /\ \A destination \in Destinations :
              parent \in DirectoryAncestors[destination] => GeneralHierarchyLeafAbsent(destination)
  BY SMT
  DEF GeneralHierarchyNext, GeneralHierarchyDirectoryAuthority, DirectoryStageOwned,
      RetainedDirectoryVars, DirectoryVars, DirectoryOwned, RetainedDirectoryInvariant,
      DirectoryInvariant, DirectoryTypeOK

THEOREM JoinedReverseSuspensionRetainsLeafUndo ==
  ASSUME NEW destination \in Destinations, GeneralHierarchyNext, DirectorySuspendLeaf(destination)
  PROVE /\ cells' = cells /\ GeneralHierarchyLeafAbsent(destination)
        /\ DirectoryBindingMatches(destination)
  BY SMT
  DEF GeneralHierarchyNext, GeneralHierarchyDirectoryAuthority, DirectorySuspendLeaf,
      RetainedDirectoryVars, DirectoryVars, retentionLifecycleVars, preparedLifecycleVars,
      activationLifecycleVars, lifecycleVars, vars

THEOREM JoinedRetainedGenerationCannotReleaseReceipt ==
  ASSUME RetainedDirectoryInvariant, NEW receipt \in retainedReceipts,
         RecoveryReceiptTarget[receipt] \in visible \union stable, GeneralHierarchyNext
  PROVE receipt \in retainedReceipts'
  BY SMT
  DEF GeneralHierarchyNext, GeneralHierarchyDirectoryAuthority, RetainedDirectoryInvariant,
      RetainedDirectoryNext, DirectoryRetainReceipt, RetainMigrationReceipt, ReleaseRecoveryReceipt,
      DirectoryStageOwned, DirectorySealReverseIntent, DirectoryReverseUnlink,
      DirectoryReinstallStage, DirectoryDiscardStage, DirectorySubstituteStage,
      DirectoryAddForeignStageChild, DirectorySuspendLeaf, DirectoryWriteEarlierRoot,
      DirectoryNormalizeRoot, RetainedDirectoryVars, DirectoryStageVars

=============================================================================
