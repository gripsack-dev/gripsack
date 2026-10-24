---- MODULE RetainedDirectoryProofs ----
EXTENDS RetainedDirectory, OwnedDirectoryProofs

THEOREM RetainedDirectoryInitialSafety == RetainedDirectoryInit => RetainedDirectoryInvariant
  BY SMT, DirectoryInitialSafety DEF RetainedDirectoryInit, RetainedDirectoryInvariant, DirectoryImages

THEOREM RetainedDirectoryBaseSafety ==
  ASSUME RetainedDirectoryInvariant, DirectoryNext, UNCHANGED DirectoryStageVars
  PROVE RetainedDirectoryInvariant'
  BY SMT, DirectoryInduction DEF RetainedDirectoryInvariant, DirectoryStageVars

THEOREM RetainedDirectoryReceiptSafety ==
  ASSUME RetainedDirectoryInvariant, NEW receipt \in RecoveryReceiptKeys,
         DirectoryRetainReceipt(receipt) \/ RetainMigrationReceipt(receipt) \/ ReleaseRecoveryReceipt(receipt)
  PROVE RetainedDirectoryInvariant'
  BY SMT DEF DirectoryRetainReceipt, RetainMigrationReceipt, ReleaseRecoveryReceipt, RetainedDirectoryInvariant,
      DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety, DirectoryVars

THEOREM RetainedDirectoryStagingSafety ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents, DirectoryStageOwned(parent)
  PROVE RetainedDirectoryInvariant'
  BY SMT, DirectoryParameters
  DEF DirectoryStageOwned, RetainedDirectoryInvariant, DirectoryInvariant, DirectoryTypeOK,
      DirectoryEvidence, DirectorySafety, DirectoryOwned, DirectoryImages

THEOREM RetainedDirectoryReverseIntentSafety ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents, DirectorySealReverseIntent(parent)
  PROVE RetainedDirectoryInvariant'
  BY SMT DEF DirectorySealReverseIntent, RetainedDirectoryInvariant, DirectoryInvariant,
      DirectoryTypeOK, DirectoryEvidence, DirectorySafety, DirectoryStageVars

THEOREM RetainedDirectoryReverseUnlinkSafety ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents, DirectoryReverseUnlink(parent)
  PROVE RetainedDirectoryInvariant'
  BY SMT, DirectoryParameters, DirectoryMatchingAncestorsHaveEvidence
  DEF DirectoryReverseUnlink, RetainedDirectoryInvariant, DirectoryInvariant, DirectoryTypeOK,
      DirectoryEvidence, DirectorySafety, DirectoryOwned, DirectoryImages, DirectoryStageVars

THEOREM RetainedDirectoryReinstallationSafety ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents, DirectoryReinstallStage(parent)
  PROVE RetainedDirectoryInvariant'
  BY SMT, DirectoryParameters
  DEF DirectoryReinstallStage, RetainedDirectoryInvariant, DirectoryInvariant, DirectoryTypeOK,
      DirectoryEvidence, DirectorySafety, DirectoryOwned, DirectoryImages

THEOREM RetainedDirectoryStageCollectionSafety ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents, DirectoryDiscardStage(parent)
  PROVE RetainedDirectoryInvariant'
  BY SMT DEF DirectoryDiscardStage, RetainedDirectoryInvariant, DirectoryInvariant,
      DirectoryTypeOK, DirectoryEvidence, DirectorySafety, DirectoryVars, DirectoryImages

THEOREM RetainedDirectoryForeignStageSafety ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents,
         DirectorySubstituteStage(parent) \/ DirectoryAddForeignStageChild(parent)
  PROVE RetainedDirectoryInvariant'
  BY SMT, DirectoryParameters
  DEF DirectorySubstituteStage, DirectoryAddForeignStageChild, RetainedDirectoryInvariant,
      DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety,
      DirectoryVars, DirectoryImages, DirectoryForeign

THEOREM RetainedDirectoryLeafSuspensionSafety ==
  ASSUME RetainedDirectoryInvariant, NEW leaf \in DirectoryLeaves, DirectorySuspendLeaf(leaf)
  PROVE RetainedDirectoryInvariant'
  BY SMT DEF DirectorySuspendLeaf, RetainedDirectoryInvariant, DirectoryInvariant,
      DirectoryTypeOK, DirectoryEvidence, DirectorySafety, DirectoryStageVars

THEOREM RetainedDirectoryRootNormalizationSafety ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents,
         DirectoryWriteEarlierRoot(parent) \/ DirectoryNormalizeRoot(parent)
  PROVE RetainedDirectoryInvariant'
  BY SMT, DirectoryParameters, DirectoryReceiptParameters
  DEF DirectoryWriteEarlierRoot, DirectoryNormalizeRoot, RetainedDirectoryInvariant,
      DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety,
      DirectoryStageVars, DirectoryRootImages, DirectoryImages, DirectoryOwned, DirectoryPrior

THEOREM RetainedDirectoryInduction ==
  RetainedDirectoryInvariant /\ RetainedDirectoryNext => RetainedDirectoryInvariant'
  BY SMT, DirectoryReceiptParameters, RetainedDirectoryBaseSafety, RetainedDirectoryReceiptSafety,
      RetainedDirectoryStagingSafety,
      RetainedDirectoryReverseIntentSafety, RetainedDirectoryReverseUnlinkSafety,
      RetainedDirectoryReinstallationSafety, RetainedDirectoryStageCollectionSafety,
      RetainedDirectoryForeignStageSafety, RetainedDirectoryLeafSuspensionSafety,
      RetainedDirectoryRootNormalizationSafety
  DEF RetainedDirectoryNext

THEOREM ArbitraryFiniteRetainedDirectorySafety == RetainedDirectorySpec => []RetainedDirectoryInvariant
<1>1. RetainedDirectoryInit => RetainedDirectoryInvariant
  BY RetainedDirectoryInitialSafety
<1>2. RetainedDirectoryInvariant /\ [RetainedDirectoryNext]_RetainedDirectoryVars => RetainedDirectoryInvariant'
  BY RetainedDirectoryInduction
  DEF RetainedDirectoryVars, DirectoryStageVars, DirectoryVars, RetainedDirectoryInvariant,
      DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety
<1>3. QED
  BY PTL, <1>1, <1>2 DEF RetainedDirectorySpec

THEOREM ReverseStagePreservesExactEmptyInode ==
  ASSUME RetainedDirectoryInvariant, NEW parent \in DirectoryParents, DirectoryStageOwned(parent)
  PROVE /\ directoryStages'[parent] = DirectoryOwned(parent)
        /\ DirectoryChildrenSettled(parent) /\ parent \notin directoryForeignChildren
  BY SMT DEF DirectoryStageOwned, RetainedDirectoryInvariant

THEOREM ReverseReinstallationPrecedesLeafAuthority ==
  ASSUME DirectoryInvariant, NEW leaf \in directoryLeaves, NEW parent \in DirectoryAncestors[leaf],
         DirectoryBindingMatches(leaf)
  PROVE directoryLive[parent] = DirectoryOwned(parent)
  BY SMT DEF DirectoryInvariant, DirectoryEvidence, DirectoryTypeOK,
      DirectoryBindingMatches, DirectoryOwned

THEOREM RetainedStageCollectionNeedsNoReceipt ==
  ASSUME NEW parent \in DirectoryParents, DirectoryDiscardStage(parent)
  PROVE /\ parent \notin retainedReceipts /\ parent \notin directoryIntents
        /\ parent \notin directoryPending /\ parent \notin directoryStageForeignChildren
        /\ directoryStages[parent] = DirectoryOwned(parent)
  BY DEF DirectoryDiscardStage

THEOREM ReverseParentMutationNeedsDurableIntent ==
  ASSUME NEW parent \in DirectoryParents,
         DirectoryReverseUnlink(parent) \/ DirectoryReinstallStage(parent)
  PROVE parent \in directoryIntents
  BY SMT DEF DirectoryReverseUnlink, DirectoryReinstallStage

THEOREM ReceiptLineageContainsPredecessorRoots ==
  ASSUME NEW receipt \in retainedReceipts, retainedReceipts \subseteq RecoveryReceiptKeys,
         NEW generation \in RecoveryReceiptLineage[receipt],
         NEW predecessor \in DirectoryGenerationPredecessors[generation]
  PROVE DirectoryGenerationRoots[predecessor] \subseteq RecoveryReceiptRoots(retainedReceipts)
  BY SMT, DirectoryReceiptParameters DEF RecoveryReceiptRoots

THEOREM ReinstalledDirectoryCannotReplayRootNormalization ==
  ASSUME NEW parent \in DirectoryParents, directoryLive[parent] = DirectoryOwned(parent)
  PROVE ~DirectoryNormalizeRoot(parent) /\ ~DirectoryWriteEarlierRoot(parent)
  BY SMT, DirectoryReceiptParameters
  DEF DirectoryNormalizeRoot, DirectoryWriteEarlierRoot, DirectoryRootImages, DirectoryOwned, DirectoryPrior

=============================================================================
