---- MODULE IncarnationEvolutionProofs ----
EXTENDS IncarnationDomain, TLAPS

THEOREM IncarnationPublishPreparedSafety ==
  ASSUME IncarnationInvariant, NEW identity \in IncarnationIds, IncarnationPublishPrepared(identity)
  PROVE IncarnationInvariant'
  BY SMT DEF IncarnationPublishPrepared, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationPublishTargetSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationPublishTarget(root)
  PROVE IncarnationInvariant'
  BY SMT DEF IncarnationPublishTarget, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationSelectDeclarationsSafety ==
  ASSUME IncarnationInvariant, NEW declarations \in SUBSET DirectoryLeaves,
         IncarnationSelectDeclarations(declarations)
  PROVE IncarnationInvariant'
  BY SMT DEF IncarnationSelectDeclarations, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationBeginEvolveSafety ==
  ASSUME IncarnationInvariant, NEW target \in [DirectoryParents -> IncarnationImages],
         NEW declarations \in SUBSET DirectoryLeaves, NEW receipt \in RecoveryReceiptKeys,
         IncarnationBeginEvolve(target, declarations, receipt)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters
  DEF IncarnationBeginEvolve, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationOwnedImages, IncarnationOwned, IncarnationSnapshotIds,
      IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationEvolveStageNodeSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationEvolveStageNode(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters
  DEF IncarnationEvolveStageNode, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationOwnedImages, IncarnationOwned, IncarnationImages,
      IncarnationBorrowedImages, IncarnationBorrowed, IncarnationForeign, DirectoryRootImages,
      IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationEvolveInstallNodeSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationEvolveInstallNode(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters
  DEF IncarnationEvolveInstallNode, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationOwnedImages, IncarnationOwned, IncarnationImages,
      IncarnationBorrowedImages, IncarnationBorrowed, IncarnationForeign, DirectoryRootImages,
      IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationRecoverEvolveSafety ==
  IncarnationInvariant /\ IncarnationRecoverEvolve => IncarnationInvariant'
  BY SMT DEF IncarnationRecoverEvolve, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationEvolveRestageTargetSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationEvolveRestageTarget(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters
  DEF IncarnationEvolveRestageTarget, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationOwnedImages, IncarnationOwned, IncarnationImages,
      IncarnationBorrowedImages, IncarnationBorrowed, IncarnationForeign, DirectoryRootImages,
      IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationEvolveRestoreNodeSafety ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationEvolveRestoreNode(root)
  PROVE IncarnationInvariant'
  BY SMT, IncarnationParameters
  DEF IncarnationEvolveRestoreNode, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationOwnedImages, IncarnationOwned, IncarnationImages,
      IncarnationBorrowedImages, IncarnationBorrowed, IncarnationForeign, DirectoryRootImages,
      IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationPublishEvolveSafety ==
  IncarnationInvariant /\ IncarnationPublishEvolve => IncarnationInvariant'
  BY SMT, IncarnationParameters
  DEF IncarnationPublishEvolve, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationSnapshotIds, IncarnationOwnedImages, IncarnationOwned,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationCommitEvolveSafety ==
  IncarnationInvariant /\ IncarnationCommitEvolve => IncarnationInvariant'
  BY SMT DEF IncarnationCommitEvolve, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM IncarnationFinishEvolveRecoverySafety ==
  IncarnationInvariant /\ IncarnationFinishEvolveRecovery => IncarnationInvariant'
  BY SMT DEF IncarnationFinishEvolveRecovery, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM EvolveBorrowedWitnessHasNoDeleteAuthority ==
  ASSUME NEW root \in DirectoryParents, incarnation.live[root] \in IncarnationBorrowedImages
  PROVE /\ ~IncarnationEvolveStageNode(root) /\ ~IncarnationEvolveRestageTarget(root)
        /\ ~IncarnationEvolveInstallNode(root) /\ ~IncarnationEvolveRestoreNode(root)
  BY SMT DEF IncarnationEvolveStageNode, IncarnationEvolveRestageTarget,
      IncarnationEvolveInstallNode, IncarnationEvolveRestoreNode, IncarnationIsOwned,
      IncarnationOwnedImages, IncarnationOwned, IncarnationBorrowedImages, IncarnationBorrowed

THEOREM EvolveNodeRetirementIsChildFirst ==
  ASSUME NEW root \in DirectoryParents,
         IncarnationEvolveStageNode(root) \/ IncarnationEvolveRestageTarget(root)
  PROVE IncarnationChildrenEmpty(root) /\ IncarnationAncestorsMatch(root)
  BY SMT DEF IncarnationEvolveStageNode, IncarnationEvolveRestageTarget

THEOREM EvolveNodeInstallationIsParentFirst ==
  ASSUME NEW root \in DirectoryParents, IncarnationEvolveInstallNode(root) \/ IncarnationEvolveRestoreNode(root)
  PROVE /\ IncarnationAncestorsMatch(root)
        /\ \A ancestor \in DirectoryAncestors[root] :
              incarnation.live[ancestor] = IF incarnation.evolveUndo
                  THEN incarnation.evolveOriginal[ancestor] ELSE incarnation.evolveTarget[ancestor]
  BY SMT DEF IncarnationEvolveInstallNode, IncarnationEvolveRestoreNode, IncarnationEvolveAncestors

THEOREM EvolvePublishedReceiptSurvivesUndo ==
  ASSUME IncarnationInvariant, incarnation.evolveActive, incarnation.evolvePublished, incarnation.evolveUndo
  PROVE IncarnationSnapshotIds(incarnation.evolveTarget) \subseteq incarnation.receipts
  BY DEF IncarnationInvariant, IncarnationEvidence

THEOREM EvolveBirthIntentPrecedesMutation ==
  ASSUME NEW root \in DirectoryParents,
         IncarnationEvolveStageNode(root) \/ IncarnationEvolveInstallNode(root)
           \/ IncarnationEvolveRestageTarget(root) \/ IncarnationEvolveRestoreNode(root)
  PROVE incarnation.evolveActive /\ root \in IncarnationEvolveChanged
  BY SMT DEF IncarnationEvolveStageNode, IncarnationEvolveInstallNode,
      IncarnationEvolveRestageTarget, IncarnationEvolveRestoreNode

THEOREM DeclarationAdmissionIsCurrent ==
  ASSUME NEW leaf \in DirectoryLeaves, IncarnationExposeLeaf(leaf)
  PROVE leaf \in incarnation.declared \union incarnation.priorDeclared
          \/ (incarnation.evolveActive /\ leaf \in incarnation.evolveOriginalDeclared)
  BY DEF IncarnationExposeLeaf, IncarnationLeafAdmissible

THEOREM StageNamesAreExclusiveOutsideHierarchy ==
  ASSUME NEW first \in IncarnationIds, NEW second \in IncarnationIds, first # second
  PROVE /\ IncarnationStageName[first] # IncarnationStageName[second]
        /\ IncarnationStageName[first] \notin DirectoryParents \union DirectoryLeaves
        /\ IncarnationStageAncestors[IncarnationStageName[first]] = {}
  BY SMT, IncarnationParameters

THEOREM EvolveBirthUsesPersistedExactIdentity ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationEvolveInstallNode(root)
  PROVE /\ incarnation.evolveActive
        /\ incarnation.evolveTarget[root][2] \in incarnation.prepared
        /\ incarnation.evolveTarget[root][2] \in incarnation.stages
        /\ IncarnationRoot[incarnation.evolveTarget[root][2]] = root
  BY SMT DEF IncarnationEvolveInstallNode, IncarnationInvariant, IncarnationEvidence

THEOREM LegacyJournalHasNoIncarnationMutationAuthority ==
  ASSUME IncarnationInvariant, DirectoryJournalVersion \in {1, 2}
  PROVE incarnation.intents = {} /\ ~incarnation.evolveActive
  BY SMT DEF IncarnationInvariant, IncarnationEvidence

THEOREM IncarnationForeignStageChildSafety ==
  ASSUME IncarnationInvariant, NEW identity \in IncarnationIds, IncarnationForeignStageChild(identity)
  PROVE IncarnationInvariant'
  BY SMT DEF IncarnationForeignStageChild, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationIsOwned, IncarnationBeforeOwned, IncarnationOldIdentity

THEOREM OriginalRestorationMovesForeignChildrenHome ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationRestoreOriginal(root),
         IncarnationBeforeOwned(root), IncarnationOldIdentity(root) \in incarnation.foreignStageChildren
  PROVE /\ incarnation'.live[root] = incarnation.before[root]
        /\ root \in incarnation'.foreignChildren
        /\ IncarnationOldIdentity(root) \notin incarnation'.foreignStageChildren
        /\ incarnation'.foreignLeaves = incarnation.foreignLeaves
        /\ incarnation'.declared = incarnation.declared /\ incarnation'.bindings = incarnation.bindings
  BY SMT DEF IncarnationRestoreOriginal, IncarnationInvariant, IncarnationTypeOK

THEOREM EvolveOriginalRestorationMovesForeignChildrenHome ==
  ASSUME IncarnationInvariant, NEW root \in DirectoryParents, IncarnationEvolveRestoreNode(root),
         incarnation.evolveOriginal[root][2] \in incarnation.foreignStageChildren
  PROVE /\ incarnation'.live[root] = incarnation.evolveOriginal[root]
        /\ root \in incarnation'.foreignChildren
        /\ incarnation.evolveOriginal[root][2] \notin incarnation'.foreignStageChildren
        /\ incarnation'.foreignLeaves = incarnation.foreignLeaves
        /\ incarnation'.declared = incarnation.declared /\ incarnation'.bindings = incarnation.bindings
  BY SMT DEF IncarnationEvolveRestoreNode, IncarnationInvariant, IncarnationTypeOK

THEOREM TargetInstallationStillRequiresEmptyStage ==
  ASSUME NEW root \in DirectoryParents
  PROVE /\ (IncarnationInstallTarget(root) =>
              incarnation.target[root] \notin incarnation.foreignStageChildren \union incarnation.foreignStages)
        /\ (IncarnationEvolveInstallNode(root) =>
              incarnation.evolveTarget[root][2] \notin incarnation.foreignStageChildren \union incarnation.foreignStages)
  BY SMT DEF IncarnationInstallTarget, IncarnationEvolveInstallNode

THEOREM StageRetirementPreservesForeignContents ==
  ASSUME NEW identity \in IncarnationIds, IncarnationDiscardStage(identity)
  PROVE identity \notin incarnation.foreignStageChildren \union incarnation.foreignStages
  BY SMT DEF IncarnationDiscardStage

=============================================================================
