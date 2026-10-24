---- MODULE IncarnationDomainProofs ----
EXTENDS IncarnationEvolutionProofs

THEOREM IncarnationInitialSafety == IncarnationInit => IncarnationInvariant
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationInit, IncarnationInitialPrepared, IncarnationInvariant, IncarnationTypeOK,
      IncarnationEvidence, IncarnationIsOwned, IncarnationOwnedImages, IncarnationOwned,
      IncarnationImages, IncarnationForeign, IncarnationBeforeOwned, IncarnationOldIdentity,
      IncarnationNone, DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationPrepareSafety ==
  ASSUME IncarnationInvariant, NEW identity \in IncarnationIds, IncarnationPrepare(identity)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationPrepare, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationBeginSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, NEW identity \in IncarnationIds,
         NEW direction \in {"forward", "reverse", "reactivate", "exchange"}, IncarnationBegin(root, identity, direction)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationBegin, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationApplyUnlinkSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationApplyUnlink(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationApplyUnlink, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationStageOriginalSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationStageOriginal(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationStageOriginal, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationPublishPredecessorSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationPublishPredecessor(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationPublishPredecessor, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationInstallTargetSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationInstallTarget(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationInstallTarget, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationExposeLeafSafety ==
  ASSUME IncarnationInvariant, NEW leaf \in DirectoryLeaves, IncarnationExposeLeaf(leaf)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationExposeLeaf, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds,
      IncarnationIsWitness

THEOREM IncarnationSuspendLeafSafety ==
  ASSUME IncarnationInvariant, NEW leaf \in DirectoryLeaves, IncarnationSuspendLeaf(leaf)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationSuspendLeaf, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationRecoverSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationRecover(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationRecover, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationWriteEarlierRootSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, NEW image \in DirectoryRootImages, IncarnationWriteEarlierRoot(root, image)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationWriteEarlierRoot, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationNormalizeRootSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationNormalizeRoot(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationNormalizeRoot, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationUndoTargetSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationUndoTarget(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationUndoTarget, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds,
      IncarnationKeepTarget

THEOREM IncarnationUndoUnlinkPredecessorSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationUndoUnlinkPredecessor(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationUndoUnlinkPredecessor, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationRestoreOriginalSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationRestoreOriginal(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationRestoreOriginal, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationFinishRecoverySafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationFinishRecovery(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationFinishRecovery, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationCommitSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationCommit(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationCommit, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationReleaseReceiptSafety ==
  ASSUME IncarnationInvariant, NEW identity \in IncarnationIds, IncarnationReleaseReceipt(identity)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationReleaseReceipt, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationDiscardStageSafety ==
  ASSUME IncarnationInvariant, NEW identity \in IncarnationIds, IncarnationDiscardStage(identity)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationDiscardStage, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationForeignRootSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationForeignRoot(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationForeignRoot, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationForeignChildSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationForeignChild(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationForeignChild, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationForeignLeafSafety ==
  ASSUME IncarnationInvariant, NEW leaf \in DirectoryLeaves, IncarnationForeignLeaf(leaf)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationForeignLeaf, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationForeignStageSafety ==
  ASSUME IncarnationInvariant, NEW identity \in IncarnationIds, IncarnationForeignStage(identity)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters, DirectoryParameters
  DEF IncarnationForeignStage, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity, IncarnationTargetImage,
      IncarnationOwned, IncarnationOwnedImages, IncarnationImages, IncarnationForeign,
      DirectoryRootImages, IncarnationBorrowedImages, IncarnationBorrowed, IncarnationSnapshotIds

THEOREM IncarnationInduction == IncarnationInvariant /\ IncarnationNext => IncarnationInvariant'
  BY SMT, IncarnationPrepareSafety,
      IncarnationBeginSafety,
      IncarnationApplyUnlinkSafety,
      IncarnationStageOriginalSafety,
      IncarnationPublishPredecessorSafety,
      IncarnationInstallTargetSafety,
      IncarnationExposeLeafSafety,
      IncarnationSuspendLeafSafety,
      IncarnationRecoverSafety,
      IncarnationWriteEarlierRootSafety,
      IncarnationNormalizeRootSafety,
      IncarnationUndoTargetSafety,
      IncarnationUndoUnlinkPredecessorSafety,
      IncarnationRestoreOriginalSafety,
      IncarnationFinishRecoverySafety,
      IncarnationCommitSafety,
      IncarnationReleaseReceiptSafety,
      IncarnationDiscardStageSafety,
      IncarnationForeignRootSafety,
      IncarnationForeignChildSafety,
      IncarnationForeignLeafSafety,
      IncarnationForeignStageSafety, IncarnationForeignStageChildSafety,
      IncarnationPublishPreparedSafety, IncarnationPublishTargetSafety,
      IncarnationSelectDeclarationsSafety,
      IncarnationBeginEvolveSafety, IncarnationEvolveStageNodeSafety, IncarnationEvolveInstallNodeSafety,
      IncarnationRecoverEvolveSafety, IncarnationEvolveRestageTargetSafety, IncarnationEvolveRestoreNodeSafety,
      IncarnationPublishEvolveSafety, IncarnationCommitEvolveSafety, IncarnationFinishEvolveRecoverySafety
  DEF IncarnationNext

THEOREM ArbitraryFiniteIncarnationSafety == IncarnationSpec => []IncarnationInvariant
<1>1. IncarnationInit => IncarnationInvariant
  BY IncarnationInitialSafety
<1>2. IncarnationInvariant /\ [IncarnationNext]_IncarnationVars => IncarnationInvariant'
  BY IncarnationInduction DEF IncarnationVars, IncarnationInvariant, IncarnationTypeOK,
      IncarnationEvidence, IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity
<1>3. QED
  BY PTL, <1>1, <1>2 DEF IncarnationSpec

THEOREM ExchangeIntentBindsBothIncarnations ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, NEW identity \in IncarnationIds,
         IncarnationBegin(root, identity, "exchange")
  PROVE /\ root \in incarnation'.intents
        /\ incarnation'.before[root] = incarnation.live[root]
        /\ incarnation'.target[root] = identity
        /\ incarnation.live[root] # IncarnationOwned(identity)
  BY SMT DEF IncarnationBegin, IncarnationInvariant, IncarnationTypeOK

THEOREM TargetInstallationRequiresDurableExchange ==
  ASSUME NEW root \in DirectoryParents, IncarnationInstallTarget(root)
  PROVE root \in incarnation.intents /\ incarnation.target[root] \in incarnation.stages
  BY DEF IncarnationInstallTarget

THEOREM RetainedTargetUndoRestagesExactInode ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationUndoTarget(root),
         IncarnationKeepTarget(root)
  PROVE incarnation.target[root] \in incarnation'.stages
  BY SMT DEF IncarnationUndoTarget, IncarnationInvariant, IncarnationTypeOK, IncarnationKeepTarget

THEOREM OriginalRestorationUsesCapturedImmediatePrior ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationRestoreOriginal(root)
  PROVE incarnation'.live[root] = incarnation.before[root]
  BY SMT DEF IncarnationRestoreOriginal, IncarnationInvariant, IncarnationTypeOK

THEOREM RootNormalizationCannotReplayAfterReinstallation ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationIsOwned(root)
  PROVE ~IncarnationNormalizeRoot(root)
  BY SMT DEF IncarnationNormalizeRoot, IncarnationInvariant, IncarnationTypeOK,
      IncarnationIsOwned, IncarnationOwnedImages, IncarnationOwned, DirectoryRootImages

THEOREM ParentEvacuationRequiresExactEmptyIdentity ==
  ASSUME NEW root \in DirectoryParents, IncarnationStageOriginal(root) \/ IncarnationUndoTarget(root)
  PROVE IncarnationChildrenEmpty(root) /\ IncarnationAncestorsMatch(root)
  BY SMT DEF IncarnationStageOriginal, IncarnationUndoTarget

THEOREM IncarnationLeafMutationBindsAncestors ==
  ASSUME NEW leaf \in DirectoryLeaves, IncarnationSuspendLeaf(leaf)
  PROVE IncarnationBindingMatches(leaf) /\ leaf \notin incarnation.foreignLeaves
  BY DEF IncarnationSuspendLeaf

THEOREM IncarnationReceiptUpdateFrame ==
  ASSUME IncarnationInvariant, IncarnationNext
  PROVE incarnation'.receipts = incarnation.receipts
        \/ (\E root \in DirectoryParents : IncarnationCommit(root))
        \/ (\E identity \in IncarnationIds : IncarnationReleaseReceipt(identity))
        \/ (\E root \in DirectoryParents : IncarnationPublishTarget(root)) \/ IncarnationPublishEvolve
        \/ (\E identity \in IncarnationIds : IncarnationPublishPrepared(identity))
  BY SMT
  DEF IncarnationNext, IncarnationPrepare, IncarnationBegin, IncarnationApplyUnlink,
      IncarnationStageOriginal, IncarnationPublishPredecessor, IncarnationInstallTarget,
      IncarnationExposeLeaf, IncarnationSuspendLeaf, IncarnationRecover,
      IncarnationWriteEarlierRoot, IncarnationNormalizeRoot, IncarnationUndoTarget,
      IncarnationUndoUnlinkPredecessor, IncarnationRestoreOriginal, IncarnationFinishRecovery,
      IncarnationDiscardStage, IncarnationForeignRoot, IncarnationForeignChild,
      IncarnationForeignLeaf, IncarnationForeignStage, IncarnationForeignStageChild,
      IncarnationInvariant, IncarnationTypeOK,
      IncarnationSelectDeclarations, IncarnationBeginEvolve, IncarnationEvolveStageNode,
      IncarnationEvolveInstallNode, IncarnationRecoverEvolve, IncarnationEvolveRestageTarget,
      IncarnationEvolveRestoreNode, IncarnationCommitEvolve, IncarnationFinishEvolveRecovery

THEOREM IncarnationParentMutationRequiresDurableIntent ==
  ASSUME NEW root \in DirectoryParents,
         IncarnationApplyUnlink(root) \/ IncarnationStageOriginal(root)
           \/ IncarnationPublishPredecessor(root) \/ IncarnationInstallTarget(root)
           \/ IncarnationNormalizeRoot(root) \/ IncarnationUndoTarget(root)
           \/ IncarnationUndoUnlinkPredecessor(root) \/ IncarnationRestoreOriginal(root)
           \/ (\E image \in DirectoryRootImages : IncarnationWriteEarlierRoot(root, image))
  PROVE root \in incarnation.intents
  BY SMT DEF IncarnationApplyUnlink, IncarnationStageOriginal, IncarnationPublishPredecessor,
      IncarnationInstallTarget, IncarnationNormalizeRoot, IncarnationUndoTarget,
      IncarnationUndoUnlinkPredecessor, IncarnationRestoreOriginal, IncarnationWriteEarlierRoot

=============================================================================
