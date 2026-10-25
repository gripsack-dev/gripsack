---- MODULE IncarnationDomain ----
EXTENDS RetainedDirectory

\* Logical roots and inode incarnations are separate domains. In particular,
\* two immutable generations may retain different inodes at the same pathname.
CONSTANTS IncarnationIds, IncarnationRoot, IncarnationPredecessor,
          IncarnationInitialLive, IncarnationInitialStages, IncarnationInitialReceipts,
          IncarnationReceiptKey, IncarnationBorrowedIds, IncarnationBorrowedRoot,
          IncarnationStagePaths, IncarnationStageName, IncarnationStageAncestors, IncarnationInitialDeclarations,
          IncarnationNoReceipt
IncarnationNone == <<"no-identity">>
NoReceipt == <<"no-receipt">>
IncarnationOwned(identity) == <<"directory", identity>>
IncarnationForeign(root) == <<"foreign", root>>
IncarnationBorrowed(identity) == <<"borrowed-directory", identity>>
IncarnationBorrowedImages == {IncarnationBorrowed(identity) : identity \in IncarnationBorrowedIds}
IncarnationOwnedImages == {IncarnationOwned(identity) : identity \in IncarnationIds}
IncarnationImages == DirectoryRootImages \union IncarnationOwnedImages \union IncarnationBorrowedImages
    \union {IncarnationForeign(root) : root \in DirectoryParents}
IncarnationInitialPrepared == IncarnationInitialStages \union
    {identity \in IncarnationIds : IncarnationInitialLive[IncarnationRoot[identity]] = IncarnationOwned(identity)}
ASSUME IncarnationParameters ==
    /\ IsFiniteSet(IncarnationIds) /\ IncarnationIds \subseteq DirectoryIdentities
    /\ IncarnationNoReceipt \notin RecoveryReceiptKeys
    /\ IncarnationBorrowedIds \subseteq DirectoryIdentities
    /\ IncarnationBorrowedIds \intersect IncarnationIds = {}
    /\ IncarnationBorrowedRoot \in [IncarnationBorrowedIds -> DirectoryParents]
    /\ IncarnationStagePaths \intersect (DirectoryParents \union DirectoryLeaves) = {}
    /\ IncarnationStageName \in [IncarnationIds -> IncarnationStagePaths]
    /\ IncarnationStageAncestors \in [IncarnationStagePaths -> SUBSET DirectoryParents]
    /\ \A stage \in IncarnationStagePaths : IncarnationStageAncestors[stage] = {}
    /\ \A first, second \in IncarnationIds :
          IncarnationStageName[first] = IncarnationStageName[second] => first = second
    /\ IncarnationInitialDeclarations \subseteq DirectoryLeaves
    /\ IncarnationRoot \in [IncarnationIds -> DirectoryParents]
    /\ IncarnationPredecessor \in [IncarnationIds -> DirectoryRootImages]
    /\ IncarnationReceiptKey \in [IncarnationIds -> RecoveryReceiptKeys]
    /\ \A first, second \in IncarnationIds :
          IncarnationReceiptKey[first] = IncarnationReceiptKey[second] => first = second
    /\ IncarnationInitialLive \in [DirectoryParents -> IncarnationImages]
    /\ IncarnationInitialStages \subseteq IncarnationIds
    /\ IncarnationInitialReceipts \subseteq IncarnationInitialPrepared
    /\ \A identity \in IncarnationInitialStages :
          IncarnationInitialLive[IncarnationRoot[identity]] # IncarnationOwned(identity)
    /\ \A root \in DirectoryParents :
          IncarnationInitialLive[root] \in IncarnationOwnedImages =>
              IncarnationRoot[IncarnationInitialLive[root][2]] = root
    /\ \A root \in DirectoryParents :
          IncarnationInitialLive[root] \in IncarnationBorrowedImages =>
              IncarnationBorrowedRoot[IncarnationInitialLive[root][2]] = root

VARIABLE incarnation
IncarnationVars == <<incarnation>>
IncarnationIsOwned(root) == incarnation.live[root] \in IncarnationOwnedImages
IncarnationIsWitness(root) == IncarnationIsOwned(root) \/ incarnation.live[root] \in IncarnationBorrowedImages
IncarnationBindingImage(identity) ==
    IF identity \in IncarnationBorrowedIds THEN IncarnationBorrowed(identity) ELSE IncarnationOwned(identity)
IncarnationAncestorsMatch(node) ==
    \A root \in DirectoryAncestors[node] : IncarnationIsWitness(root)
IncarnationBindingMatches(leaf) ==
    \A root \in DirectoryAncestors[leaf] :
          incarnation.live[root] = IncarnationBindingImage(incarnation.bindings[leaf][root])
IncarnationChildrenEmpty(root) ==
    /\ root \notin incarnation.foreignChildren
    /\ \A leaf \in incarnation.leaves \union incarnation.foreignLeaves : root \notin DirectoryAncestors[leaf]
    /\ \A child \in DirectoryParents :
          root \in DirectoryAncestors[child] => incarnation.live[child] = <<"absent">>
IncarnationBeforeOwned(root) == incarnation.before[root] \in IncarnationOwnedImages
IncarnationOldIdentity(root) == incarnation.before[root][2]
IncarnationTargetImage(root) == IncarnationOwned(incarnation.target[root])
IncarnationSnapshotIds(snapshot) == {identity \in IncarnationIds :
    snapshot[IncarnationRoot[identity]] = IncarnationOwned(identity)}
IncarnationEvolveChanged == {root \in DirectoryParents :
    incarnation.evolveOriginal[root] # incarnation.evolveTarget[root]}
IncarnationEvolveAncestors(root, snapshot) ==
    /\ IncarnationAncestorsMatch(root)
    /\ \A ancestor \in DirectoryAncestors[root] : incarnation.live[ancestor] = snapshot[ancestor]
IncarnationLeafAdmissible(leaf) ==
    leaf \in incarnation.declared \union incarnation.priorDeclared
      \/ (incarnation.evolveActive /\ leaf \in incarnation.evolveOriginalDeclared)
IncarnationKeepTarget(root) ==
    incarnation.direction[root] # "forward" \/ incarnation.target[root] \in incarnation.receipts
IncarnationInit ==
    incarnation = [live |-> IncarnationInitialLive, stages |-> IncarnationInitialStages,
      prepared |-> IncarnationInitialPrepared, receipts |-> IncarnationInitialReceipts,
      intents |-> {}, before |-> IncarnationInitialLive,
      target |-> [root \in DirectoryParents |-> IncarnationNone],
      direction |-> [root \in DirectoryParents |-> "forward"],
      phase |-> [root \in DirectoryParents |-> "apply"],
      rootAfter |-> [root \in DirectoryParents |-> <<"absent">>],
      leaves |-> {}, bindings |-> [leaf \in DirectoryLeaves |->
          [root \in DirectoryParents |-> IncarnationNone]],
      foreignRoots |-> {root \in DirectoryParents : IncarnationInitialLive[root] = IncarnationForeign(root)},
      foreignChildren |-> {}, foreignLeaves |-> {}, foreignStages |-> {}, foreignStageChildren |-> {},
      declared |-> IncarnationInitialDeclarations, priorDeclared |-> IncarnationInitialDeclarations,
      evolveActive |-> FALSE, evolveUndo |-> FALSE, evolvePublished |-> FALSE,
      evolveOriginal |-> IncarnationInitialLive, evolveTarget |-> IncarnationInitialLive,
      evolveOriginalDeclared |-> IncarnationInitialDeclarations, evolveReceipt |-> IncarnationNoReceipt]
IncarnationPrepare(identity) ==
    /\ identity \notin incarnation.prepared
    /\ identity \notin incarnation.foreignStages
    /\ incarnation' = [incarnation EXCEPT !.prepared = @ \union {identity}, !.stages = @ \union {identity}]
\* The durable intent records BOTH identities before any exchange mutation.
IncarnationBegin(root, identity, direction) ==
    /\ DirectoryJournalVersion = 3
    /\ ~incarnation.evolveActive
    /\ root \notin incarnation.intents /\ IncarnationRoot[identity] = root
    /\ identity \in incarnation.prepared /\ IncarnationAncestorsMatch(root)
    /\ CASE direction = "forward" ->
              /\ incarnation.live[root] \in DirectoryRootImages
              /\ identity \in incarnation.stages
            [] direction = "reactivate" ->
              /\ incarnation.live[root] \in DirectoryRootImages
              /\ identity \in incarnation.stages \intersect incarnation.receipts
            [] direction = "reverse" -> incarnation.live[root] = IncarnationOwned(identity)
            [] direction = "exchange" ->
              /\ IncarnationIsOwned(root) /\ incarnation.live[root] # IncarnationOwned(identity)
              /\ identity \in incarnation.stages \intersect incarnation.receipts
              /\ incarnation.live[root][2] \in incarnation.receipts
    /\ incarnation' = [incarnation EXCEPT !.intents = @ \union {root},
          !.before[root] = incarnation.live[root], !.target[root] = identity,
          !.direction[root] = direction, !.phase[root] = "apply",
          !.rootAfter[root] = IF direction = "reverse" THEN IncarnationPredecessor[identity] ELSE <<"absent">>]
IncarnationApplyUnlink(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "apply"
    /\ incarnation.direction[root] \in {"forward", "reactivate"}
    /\ incarnation.live[root] = incarnation.before[root] /\ incarnation.live[root] \in DirectoryRootImages
    /\ IncarnationAncestorsMatch(root)
    /\ incarnation' = [incarnation EXCEPT !.live[root] = <<"absent">>]
IncarnationStageOriginal(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "apply"
    /\ incarnation.direction[root] \in {"reverse", "exchange"}
    /\ IncarnationBeforeOwned(root) /\ incarnation.live[root] = incarnation.before[root]
    /\ IncarnationOldIdentity(root) \notin incarnation.foreignStages
    /\ IncarnationChildrenEmpty(root) /\ IncarnationAncestorsMatch(root)
    /\ incarnation' = [incarnation EXCEPT !.live[root] = <<"absent">>,
          !.stages = @ \union {IncarnationOldIdentity(root)}]
IncarnationPublishPredecessor(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "apply"
    /\ incarnation.direction[root] = "reverse" /\ IncarnationBeforeOwned(root)
    /\ incarnation.live[root] = <<"absent">> /\ IncarnationOldIdentity(root) \in incarnation.stages
    /\ IncarnationAncestorsMatch(root)
    /\ incarnation' = [incarnation EXCEPT !.live[root] = IncarnationPredecessor[IncarnationOldIdentity(root)]]
IncarnationInstallTarget(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "apply"
    /\ incarnation.direction[root] \in {"forward", "reactivate", "exchange"}
    /\ incarnation.live[root] = <<"absent">> /\ incarnation.target[root] \in incarnation.stages
    /\ incarnation.target[root] \notin incarnation.foreignStages /\ IncarnationAncestorsMatch(root)
    /\ incarnation.target[root] \notin incarnation.foreignStageChildren
    /\ incarnation' = [incarnation EXCEPT !.live[root] = IncarnationTargetImage(root),
          !.stages = @ \ {incarnation.target[root]}]
IncarnationExposeLeaf(leaf) ==
    /\ IncarnationAncestorsMatch(leaf) /\ leaf \notin incarnation.foreignLeaves
    /\ IncarnationLeafAdmissible(leaf)
    /\ incarnation' = [incarnation EXCEPT !.leaves = @ \union {leaf},
          !.bindings[leaf] = [root \in DirectoryParents |->
              IF IncarnationIsWitness(root) THEN incarnation.live[root][2] ELSE IncarnationNone]]
IncarnationSuspendLeaf(leaf) ==
    /\ leaf \in incarnation.leaves /\ IncarnationBindingMatches(leaf)
    /\ leaf \notin incarnation.foreignLeaves
    /\ incarnation' = [incarnation EXCEPT !.leaves = @ \ {leaf}]
IncarnationRecover(root) ==
    /\ root \in incarnation.intents
    /\ incarnation' = [incarnation EXCEPT !.phase[root] = "undo"]
\* The ordinary root journal may have selected an older link, file or absence.
\* These acknowledged changes are synchronized with its actual durable cell.
IncarnationWriteEarlierRoot(root, image) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "apply"
    /\ incarnation.direction[root] = "reverse" /\ IncarnationBeforeOwned(root)
    /\ incarnation.live[root] = incarnation.rootAfter[root]
    /\ IncarnationOldIdentity(root) \in incarnation.stages /\ IncarnationAncestorsMatch(root)
    /\ incarnation' = [incarnation EXCEPT !.live[root] = image, !.rootAfter[root] = image]
IncarnationNormalizeRoot(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "undo"
    /\ incarnation.direction[root] = "reverse" /\ IncarnationBeforeOwned(root)
    /\ incarnation.live[root] = incarnation.rootAfter[root]
    /\ IncarnationOldIdentity(root) \in incarnation.stages /\ IncarnationAncestorsMatch(root)
    /\ incarnation' = [incarnation EXCEPT !.live[root] = IncarnationPredecessor[IncarnationOldIdentity(root)]]
\* Only an unreferenced fresh Forward may destroy its new inode. Publication,
\* even before selection, grants immutable receipt authority requiring restage.
IncarnationUndoTarget(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "undo"
    /\ incarnation.direction[root] \in {"forward", "reactivate", "exchange"}
    /\ incarnation.live[root] = IncarnationTargetImage(root)
    /\ IncarnationChildrenEmpty(root) /\ IncarnationAncestorsMatch(root)
    /\ ~IncarnationKeepTarget(root) \/ incarnation.target[root] \notin incarnation.foreignStages
    /\ incarnation' = [incarnation EXCEPT !.live[root] = <<"absent">>,
          !.stages = IF IncarnationKeepTarget(root) THEN @ \union {incarnation.target[root]} ELSE @]
IncarnationUndoUnlinkPredecessor(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "undo"
    /\ incarnation.direction[root] = "reverse" /\ IncarnationBeforeOwned(root)
    /\ incarnation.live[root] = IncarnationPredecessor[IncarnationOldIdentity(root)]
    /\ IncarnationAncestorsMatch(root)
    /\ incarnation' = [incarnation EXCEPT !.live[root] = <<"absent">>]
IncarnationRestoreOriginal(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "undo"
    /\ incarnation.live[root] = <<"absent">> /\ IncarnationAncestorsMatch(root)
    /\ IF IncarnationBeforeOwned(root) THEN
          /\ IncarnationOldIdentity(root) \in incarnation.stages
          /\ IncarnationOldIdentity(root) \notin incarnation.foreignStages
       ELSE incarnation.before[root] \in DirectoryRootImages
    \* Compensation moves the exact ORIGINAL container with all its contents.
    \* The absent destination is NoReplace authority, not permission to clear it.
    /\ incarnation' = [incarnation EXCEPT !.live[root] = incarnation.before[root],
          !.stages = IF IncarnationBeforeOwned(root) THEN @ \ {IncarnationOldIdentity(root)} ELSE @,
          !.foreignChildren = IF IncarnationBeforeOwned(root) /\ IncarnationOldIdentity(root) \in incarnation.foreignStageChildren
                              THEN @ \union {root} ELSE @,
          !.foreignStageChildren = IF IncarnationBeforeOwned(root) THEN @ \ {IncarnationOldIdentity(root)} ELSE @]
IncarnationFinishRecovery(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "undo"
    /\ incarnation.live[root] = incarnation.before[root]
    /\ incarnation' = [incarnation EXCEPT !.intents = @ \ {root}]
IncarnationCommit(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "apply"
    /\ IF incarnation.direction[root] = "reverse"
       THEN incarnation.live[root] = incarnation.rootAfter[root]
       ELSE incarnation.live[root] = IncarnationTargetImage(root)
    /\ incarnation' = [incarnation EXCEPT !.intents = @ \ {root},
          !.receipts = @ \union {incarnation.target[root]}]
\* A generation may be durably published before its directory intent exists.
\* Its prepared node then already has retention authority independent of selection.
IncarnationPublishPrepared(identity) ==
    /\ identity \in incarnation.prepared
    /\ incarnation.live[IncarnationRoot[identity]] = IncarnationOwned(identity)
          \/ identity \in incarnation.stages \ incarnation.foreignStages
    /\ incarnation' = [incarnation EXCEPT !.receipts = @ \union {identity}]
IncarnationPublishTarget(root) ==
    /\ root \in incarnation.intents /\ incarnation.phase[root] = "apply"
    /\ incarnation.live[root] = IncarnationTargetImage(root)
          \/ incarnation.target[root] \in incarnation.stages \ incarnation.foreignStages
    /\ incarnation' = [incarnation EXCEPT !.receipts = @ \union {incarnation.target[root]}]
IncarnationReleaseReceipt(identity) ==
    /\ \A root \in incarnation.intents :
          incarnation.target[root] # identity /\ incarnation.before[root] # IncarnationOwned(identity)
    /\ ~incarnation.evolveActive \/ identity \notin
          (IncarnationSnapshotIds(incarnation.evolveOriginal) \union IncarnationSnapshotIds(incarnation.evolveTarget))
    /\ incarnation' = [incarnation EXCEPT !.receipts = @ \ {identity}]
IncarnationDiscardStage(identity) ==
    /\ identity \in incarnation.stages \ incarnation.receipts /\ identity \notin incarnation.foreignStages
    /\ identity \notin incarnation.foreignStageChildren
    /\ \A root \in incarnation.intents :
          incarnation.target[root] # identity /\ incarnation.before[root] # IncarnationOwned(identity)
    /\ ~incarnation.evolveActive \/ identity \notin
          (IncarnationSnapshotIds(incarnation.evolveOriginal) \union IncarnationSnapshotIds(incarnation.evolveTarget))
    /\ incarnation' = [incarnation EXCEPT !.stages = @ \ {identity}]
\* Declarations range over an arbitrary finite universe, not the leaf list
\* captured by the first cutover. Retargeting remains an ordinary leaf update.
IncarnationSelectDeclarations(declarations) ==
    /\ ~incarnation.evolveActive
    /\ incarnation' = [incarnation EXCEPT !.priorDeclared = incarnation.declared, !.declared = declarations]
IncarnationBeginEvolve(target, declarations, receipt) ==
    /\ DirectoryJournalVersion = 3 /\ ~incarnation.evolveActive /\ incarnation.intents = {}
    /\ \A root \in DirectoryParents :
          /\ (target[root] \in IncarnationOwnedImages =>
                /\ target[root][2] \in incarnation.prepared /\ IncarnationRoot[target[root][2]] = root)
          /\ (target[root] # incarnation.live[root] =>
                /\ incarnation.live[root] = <<"absent">> \/ IncarnationIsOwned(root)
                /\ target[root] = <<"absent">> \/ target[root] \in IncarnationOwnedImages
                /\ (target[root] \in IncarnationOwnedImages =>
                      target[root][2] \in incarnation.stages \ incarnation.foreignStages))
    /\ incarnation' = [incarnation EXCEPT !.evolveActive = TRUE, !.evolveUndo = FALSE,
          !.evolvePublished = FALSE, !.evolveOriginal = incarnation.live, !.evolveTarget = target,
          !.evolveOriginalDeclared = incarnation.declared, !.priorDeclared = incarnation.declared,
          !.declared = declarations, !.evolveReceipt = receipt]
IncarnationEvolveStageNode(root) ==
    /\ incarnation.evolveActive /\ ~incarnation.evolveUndo /\ root \in IncarnationEvolveChanged
    /\ incarnation.live[root] = incarnation.evolveOriginal[root] /\ IncarnationIsOwned(root)
    /\ incarnation.live[root][2] \notin incarnation.foreignStages
    /\ IncarnationChildrenEmpty(root) /\ IncarnationAncestorsMatch(root)
    /\ incarnation' = [incarnation EXCEPT !.stages = @ \union {incarnation.live[root][2]},
          !.live[root] = <<"absent">>]
IncarnationEvolveInstallNode(root) ==
    /\ incarnation.evolveActive /\ ~incarnation.evolveUndo /\ root \in IncarnationEvolveChanged
    /\ incarnation.live[root] = <<"absent">> /\ incarnation.evolveTarget[root] \in IncarnationOwnedImages
    /\ incarnation.evolveTarget[root][2] \in incarnation.stages \ incarnation.foreignStages
    /\ incarnation.evolveTarget[root][2] \notin incarnation.foreignStageChildren
    /\ IncarnationEvolveAncestors(root, incarnation.evolveTarget)
    /\ incarnation' = [incarnation EXCEPT !.live[root] = incarnation.evolveTarget[root],
          !.stages = @ \ {incarnation.evolveTarget[root][2]}]
IncarnationRecoverEvolve ==
    /\ incarnation.evolveActive
    /\ incarnation' = [incarnation EXCEPT !.evolveUndo = TRUE]
IncarnationEvolveRestageTarget(root) ==
    /\ incarnation.evolveActive /\ incarnation.evolveUndo /\ root \in IncarnationEvolveChanged
    /\ incarnation.live[root] = incarnation.evolveTarget[root] /\ IncarnationIsOwned(root)
    /\ incarnation.live[root][2] \notin incarnation.foreignStages
    /\ IncarnationChildrenEmpty(root) /\ IncarnationAncestorsMatch(root)
    /\ incarnation' = [incarnation EXCEPT !.stages = @ \union {incarnation.live[root][2]},
          !.live[root] = <<"absent">>]
IncarnationEvolveRestoreNode(root) ==
    /\ incarnation.evolveActive /\ incarnation.evolveUndo /\ root \in IncarnationEvolveChanged
    /\ incarnation.live[root] = <<"absent">> /\ incarnation.evolveOriginal[root] \in IncarnationOwnedImages
    /\ incarnation.evolveOriginal[root][2] \in incarnation.stages \ incarnation.foreignStages
    /\ IncarnationEvolveAncestors(root, incarnation.evolveOriginal)
    /\ incarnation' = [incarnation EXCEPT !.live[root] = incarnation.evolveOriginal[root],
          !.stages = @ \ {incarnation.evolveOriginal[root][2]},
          !.foreignChildren = IF incarnation.evolveOriginal[root][2] \in incarnation.foreignStageChildren
                              THEN @ \union {root} ELSE @,
          !.foreignStageChildren = @ \ {incarnation.evolveOriginal[root][2]}]
IncarnationPublishEvolve ==
    /\ incarnation.evolveActive /\ ~incarnation.evolveUndo
    /\ \A root \in DirectoryParents :
          /\ (incarnation.evolveTarget[root] \in IncarnationOwnedImages =>
                incarnation.live[root] = incarnation.evolveTarget[root]
                  \/ incarnation.evolveTarget[root][2] \in incarnation.stages \ incarnation.foreignStages)
          /\ (incarnation.evolveTarget[root] \in IncarnationBorrowedImages =>
                incarnation.live[root] = incarnation.evolveTarget[root])
    /\ incarnation' = [incarnation EXCEPT !.evolvePublished = TRUE,
          !.receipts = @ \union IncarnationSnapshotIds(incarnation.evolveTarget)]
IncarnationCommitEvolve ==
    /\ incarnation.evolveActive /\ ~incarnation.evolveUndo /\ incarnation.evolvePublished
    /\ \A root \in IncarnationEvolveChanged : incarnation.live[root] = incarnation.evolveTarget[root]
    /\ incarnation' = [incarnation EXCEPT !.evolveActive = FALSE]
IncarnationFinishEvolveRecovery ==
    /\ incarnation.evolveActive /\ incarnation.evolveUndo
    /\ \A root \in IncarnationEvolveChanged : incarnation.live[root] = incarnation.evolveOriginal[root]
    /\ incarnation' = [incarnation EXCEPT !.evolveActive = FALSE,
          !.declared = incarnation.evolveOriginalDeclared, !.priorDeclared = incarnation.evolveOriginalDeclared]
IncarnationForeignRoot(root) ==
    incarnation' = [incarnation EXCEPT !.live[root] = IncarnationForeign(root), !.foreignRoots = @ \union {root}]
IncarnationForeignChild(root) ==
    incarnation' = [incarnation EXCEPT !.foreignChildren = @ \union {root}]
IncarnationForeignLeaf(leaf) ==
    incarnation' = [incarnation EXCEPT !.foreignLeaves = @ \union {leaf}]
IncarnationForeignStage(identity) ==
    incarnation' = [incarnation EXCEPT !.foreignStages = @ \union {identity}]
IncarnationForeignStageChild(identity) ==
    /\ identity \in incarnation.stages /\ identity \notin incarnation.foreignStages
    /\ incarnation' = [incarnation EXCEPT !.foreignStageChildren = @ \union {identity}]
IncarnationNext ==
    \/ \E identity \in IncarnationIds : IncarnationPrepare(identity) \/ IncarnationReleaseReceipt(identity)
          \/ IncarnationDiscardStage(identity) \/ IncarnationForeignStage(identity)
          \/ IncarnationPublishPrepared(identity)
          \/ IncarnationForeignStageChild(identity)
    \/ \E root \in DirectoryParents, identity \in IncarnationIds,
          direction \in {"forward", "reverse", "reactivate", "exchange"} : IncarnationBegin(root, identity, direction)
    \/ \E root \in DirectoryParents : IncarnationApplyUnlink(root) \/ IncarnationStageOriginal(root)
          \/ IncarnationPublishPredecessor(root) \/ IncarnationInstallTarget(root) \/ IncarnationRecover(root)
          \/ IncarnationNormalizeRoot(root) \/ IncarnationUndoTarget(root)
          \/ IncarnationUndoUnlinkPredecessor(root) \/ IncarnationRestoreOriginal(root)
          \/ IncarnationFinishRecovery(root) \/ IncarnationCommit(root)
          \/ IncarnationPublishTarget(root)
          \/ IncarnationEvolveStageNode(root) \/ IncarnationEvolveInstallNode(root)
          \/ IncarnationEvolveRestageTarget(root) \/ IncarnationEvolveRestoreNode(root)
          \/ IncarnationForeignRoot(root) \/ IncarnationForeignChild(root)
    \/ \E root \in DirectoryParents, image \in DirectoryRootImages : IncarnationWriteEarlierRoot(root, image)
    \/ \E leaf \in DirectoryLeaves : IncarnationExposeLeaf(leaf) \/ IncarnationSuspendLeaf(leaf) \/ IncarnationForeignLeaf(leaf)
    \/ \E declarations \in SUBSET DirectoryLeaves : IncarnationSelectDeclarations(declarations)
    \/ \E target \in [DirectoryParents -> IncarnationImages], declarations \in SUBSET DirectoryLeaves,
          receipt \in RecoveryReceiptKeys : IncarnationBeginEvolve(target, declarations, receipt)
    \/ IncarnationRecoverEvolve \/ IncarnationPublishEvolve \/ IncarnationCommitEvolve \/ IncarnationFinishEvolveRecovery
IncarnationSpec == IncarnationInit /\ [][IncarnationNext]_IncarnationVars
IncarnationTypeOK == incarnation \in [
    live: [DirectoryParents -> IncarnationImages], stages: SUBSET IncarnationIds,
    prepared: SUBSET IncarnationIds, receipts: SUBSET IncarnationIds, intents: SUBSET DirectoryParents,
    before: [DirectoryParents -> IncarnationImages],
    target: [DirectoryParents -> IncarnationIds \union {IncarnationNone}],
    direction: [DirectoryParents -> {"forward", "reverse", "reactivate", "exchange"}],
    phase: [DirectoryParents -> {"apply", "undo"}], rootAfter: [DirectoryParents -> DirectoryRootImages],
    leaves: SUBSET DirectoryLeaves,
    bindings: [DirectoryLeaves -> [DirectoryParents -> IncarnationIds \union IncarnationBorrowedIds \union {IncarnationNone}]],
    foreignRoots: SUBSET DirectoryParents, foreignChildren: SUBSET DirectoryParents,
    foreignLeaves: SUBSET DirectoryLeaves, foreignStages: SUBSET IncarnationIds,
    foreignStageChildren: SUBSET IncarnationIds,
    declared: SUBSET DirectoryLeaves, priorDeclared: SUBSET DirectoryLeaves,
    evolveActive: BOOLEAN, evolveUndo: BOOLEAN, evolvePublished: BOOLEAN,
    evolveOriginal: [DirectoryParents -> IncarnationImages], evolveTarget: [DirectoryParents -> IncarnationImages],
    evolveOriginalDeclared: SUBSET DirectoryLeaves, evolveReceipt: RecoveryReceiptKeys \union {IncarnationNoReceipt}]
IncarnationEvidence ==
    /\ incarnation.stages \subseteq incarnation.prepared /\ incarnation.receipts \subseteq incarnation.prepared
    /\ incarnation.foreignStageChildren \subseteq incarnation.stages
    /\ \A root \in DirectoryParents : IncarnationIsOwned(root) =>
          /\ incarnation.live[root][2] \in incarnation.prepared
          /\ IncarnationRoot[incarnation.live[root][2]] = root
    /\ \A root \in DirectoryParents : incarnation.live[root] \in IncarnationBorrowedImages =>
          IncarnationBorrowedRoot[incarnation.live[root][2]] = root
    /\ \A identity \in incarnation.stages : incarnation.live[IncarnationRoot[identity]] # IncarnationOwned(identity)
    /\ \A root \in incarnation.intents :
          /\ incarnation.target[root] \in incarnation.prepared /\ IncarnationRoot[incarnation.target[root]] = root
          /\ (IncarnationBeforeOwned(root) =>
                /\ IncarnationOldIdentity(root) \in incarnation.prepared
                /\ IncarnationRoot[IncarnationOldIdentity(root)] = root)
    /\ \A root \in incarnation.foreignRoots : incarnation.live[root] = IncarnationForeign(root)
    /\ (incarnation.evolveActive =>
          /\ incarnation.intents = {} /\ incarnation.evolveReceipt \in RecoveryReceiptKeys
          /\ \A root \in DirectoryParents :
                /\ (incarnation.evolveOriginal[root] \in IncarnationOwnedImages =>
                      /\ incarnation.evolveOriginal[root][2] \in incarnation.prepared
                      /\ IncarnationRoot[incarnation.evolveOriginal[root][2]] = root)
                /\ (incarnation.evolveTarget[root] \in IncarnationOwnedImages =>
                      /\ incarnation.evolveTarget[root][2] \in incarnation.prepared
                      /\ IncarnationRoot[incarnation.evolveTarget[root][2]] = root))
    /\ (incarnation.evolveActive /\ incarnation.evolvePublished =>
          IncarnationSnapshotIds(incarnation.evolveTarget) \subseteq incarnation.receipts)
    /\ ((incarnation.intents # {} \/ incarnation.evolveActive) => DirectoryJournalVersion = 3)
IncarnationInvariant == IncarnationTypeOK /\ IncarnationEvidence
=============================================================================
