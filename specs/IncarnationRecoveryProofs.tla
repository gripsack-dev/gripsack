---- MODULE IncarnationRecoveryProofs ----
EXTENDS GeneralRecoveryProofs, IncarnationDomainProofs

CONSTANTS IncarnationRootDestination, IncarnationObject, RecoveryInitialReceipts
ASSUME IncarnationCorrespondence ==
    /\ IncarnationRootDestination \in [DirectoryParents -> Destinations]
    /\ \A root \in DirectoryParents :
          DirectoryAncestors[IncarnationRootDestination[root]] = DirectoryAncestors[root]
    /\ IncarnationObject \in [DirectoryRootImages -> Objects]
    /\ IncarnationObject[<<"absent">>] = Absent
    /\ \A first, second \in DirectoryRootImages : IncarnationObject[first] = IncarnationObject[second] => first = second
    /\ RecoveryInitialReceipts \subseteq RecoveryReceiptKeys
\* Admission validates the existing immutable receipt inventory against the
\* already durable payload store. No future preservation is assumed here.
ASSUME IncarnationInitialAdmission ==
    /\ RecoveryReceiptRoots(RecoveryInitialReceipts \union
          {IncarnationReceiptKey[identity] : identity \in IncarnationInitialReceipts}) \subseteq InitialPayloads
    /\ \A identity \in IncarnationIds :
          IncarnationReceiptKey[identity] \in RecoveryInitialReceipts => identity \in IncarnationInitialReceipts

IncarnationLeafChanges(destination) ==
    cells'[destination].cached.live # cells[destination].cached.live
      \/ cells'[destination].durable.live # cells[destination].durable.live
IncarnationRestorationReady(destination) ==
    /\ \A root \in DirectoryAncestors[destination] \intersect incarnation.intents :
          incarnation.phase[root] = "undo" =>
              IF IncarnationBeforeOwned(root) THEN incarnation.live[root] = incarnation.before[root]
              ELSE cells[destination].original = Absent
    /\ (incarnation.evolveActive /\ incarnation.evolveUndo =>
          \A root \in DirectoryAncestors[destination] :
              IF incarnation.evolveOriginal[root] \in IncarnationOwnedImages \union IncarnationBorrowedImages
              THEN incarnation.live[root] = incarnation.evolveOriginal[root]
              ELSE cells[destination].original = Absent)
IncarnationJournalAuthority ==
    /\ \A destination \in Destinations :
          ((WriterStep(destination) \/ RestoreStep(destination)) /\ IncarnationLeafChanges(destination)) =>
              /\ destination \in incarnation.leaves /\ IncarnationBindingMatches(destination)
              /\ (RestoreStep(destination) => IncarnationRestorationReady(destination))
              /\ (WriterStep(destination) =>
                    destination \in incarnation.declared
                      \/ (destination \in incarnation.priorDeclared
                          /\ cells'[destination].cached.live.value = Absent))
    /\ \A root \in DirectoryParents :
          ((WriterStep(IncarnationRootDestination[root]) \/ RestoreStep(IncarnationRootDestination[root]))
           /\ IncarnationLeafChanges(IncarnationRootDestination[root])) => ~IncarnationIsWitness(root)
IncarnationRootNeedsSynchronization(root) ==
    /\ root \in incarnation.intents /\ incarnation.direction[root] = "reverse"
    /\ IncarnationBeforeOwned(root) /\ IncarnationOldIdentity(root) \in incarnation.stages
    /\ WriterStep(IncarnationRootDestination[root]) \/ RestoreStep(IncarnationRootDestination[root])
    /\ cells'[IncarnationRootDestination[root]].durable.live # cells[IncarnationRootDestination[root]].durable.live
IncarnationRootCellCorrespondence(root) ==
    /\ incarnation.live[root] \in DirectoryRootImages /\ incarnation'.live[root] \in DirectoryRootImages
    /\ cells[IncarnationRootDestination[root]].durable.live.value = IncarnationObject[incarnation.live[root]]
    /\ cells'[IncarnationRootDestination[root]].cached.live.value = IncarnationObject[incarnation'.live[root]]
    /\ cells'[IncarnationRootDestination[root]].durable.live.value = IncarnationObject[incarnation'.live[root]]
IncarnationProtectedGenerationAuthority ==
    /\ GeneralHierarchyCollectionAuthority
    /\ \A root \in incarnation.intents :
          incarnation.target[root] \in incarnation.receipts =>
              ~RemoveCollectedGeneration(RecoveryReceiptTarget[IncarnationReceiptKey[incarnation.target[root]]])
    /\ (incarnation.evolveActive /\ incarnation.evolvePublished =>
          ~RemoveCollectedGeneration(RecoveryReceiptTarget[incarnation.evolveReceipt]))
IncarnationRootJournalStep(root) ==
    /\ RetentionLifecycleNext(TRUE) /\ IncarnationProtectedGenerationAuthority /\ IncarnationJournalAuthority
    /\ UNCHANGED RetainedDirectoryVars
    /\ IncarnationRootCellCorrespondence(root)
    /\ ((\E image \in DirectoryRootImages : WriterStep(IncarnationRootDestination[root])
            /\ IncarnationWriteEarlierRoot(root, image))
        \/ (RestoreStep(IncarnationRootDestination[root]) /\ IncarnationNormalizeRoot(root)))

IncarnationDescendantsAbsent(root) ==
    \A destination \in Destinations : root \in DirectoryAncestors[destination] => GeneralHierarchyLeafAbsent(destination)
IncarnationJournalsSettled(root) ==
    /\ GeneralHierarchyLeafSettled(IncarnationRootDestination[root])
    /\ \A destination \in Destinations :
          root \in DirectoryAncestors[destination] => GeneralHierarchyLeafSettled(destination)
IncarnationDirectoryAuthority ==
    /\ \A leaf \in DirectoryLeaves : IncarnationSuspendLeaf(leaf) => GeneralHierarchyLeafAbsent(leaf)
    /\ \A root \in DirectoryParents :
          /\ ((IncarnationStageOriginal(root) \/ IncarnationUndoTarget(root)
               \/ IncarnationEvolveStageNode(root) \/ IncarnationEvolveRestageTarget(root)) =>
                  IncarnationDescendantsAbsent(root))
          /\ ((IncarnationEvolveInstallNode(root) \/ IncarnationEvolveRestoreNode(root)) =>
                GeneralHierarchyLeafAbsent(IncarnationRootDestination[root]))
          /\ (IncarnationFinishRecovery(root) => IncarnationJournalsSettled(root))
          /\ (IncarnationUndoUnlinkPredecessor(root) =>
                /\ cells[IncarnationRootDestination[root]].cached.live.value = IncarnationObject[incarnation.live[root]]
                /\ cells[IncarnationRootDestination[root]].durable.live.value = IncarnationObject[incarnation.live[root]])
          /\ ~IncarnationCommit(root) /\ ~IncarnationNormalizeRoot(root) /\ ~IncarnationPublishTarget(root)
          /\ ~(\E image \in DirectoryRootImages : IncarnationWriteEarlierRoot(root, image))
    /\ \A identity \in IncarnationIds :
          /\ (IncarnationReleaseReceipt(identity) => IncarnationReceiptKey[identity] \notin retainedReceipts)
          /\ ~IncarnationPublishPrepared(identity)
    /\ ~IncarnationPublishEvolve
    /\ (IncarnationCommitEvolve => Committed)
    /\ (IncarnationFinishEvolveRecovery => \A destination \in Destinations : GeneralHierarchyLeafSettled(destination))

\* Exchange cleanup removes only the exact currently owned target leaf. It
\* records the resulting observed absence through the existing leaf kernel;
\* the original undo record is untouched and restores only under the old inode.
IncarnationClearTargetLeaf(destination) ==
    /\ destination \in incarnation.leaves /\ IncarnationBindingMatches(destination)
    /\ ((\E root \in DirectoryAncestors[destination] \intersect incarnation.intents :
            /\ incarnation.phase[root] = "undo" /\ incarnation.direction[root] \in {"forward", "reactivate", "exchange"}
            /\ incarnation.live[root] = IncarnationTargetImage(root))
        \/ (incarnation.evolveActive /\ incarnation.evolveUndo /\
              \E root \in DirectoryAncestors[destination] \intersect IncarnationEvolveChanged :
                  incarnation.live[root] = incarnation.evolveTarget[root] /\ IncarnationIsOwned(root)))
    /\ cells[destination].cached.entry.present /\ cells[destination].durable.entry.present
    /\ cells[destination].cached.live.owner = "core"
    /\ cells[destination].cached.live.value = cells[destination].cached.entry.after
    /\ cells[destination].durable.live = cells[destination].cached.live
    /\ cells[destination].durable.entry = cells[destination].cached.entry
    /\ RetentionLifecycleNext(TRUE) /\ ExternalDestination(destination, Absent)
    /\ IncarnationProtectedGenerationAuthority /\ IncarnationSuspendLeaf(destination)
    /\ UNCHANGED RetainedDirectoryVars

IncarnationCommitReceipt(root) ==
    /\ Committed /\ IncarnationCommit(root)
    /\ RecoveryReceiptTarget[IncarnationReceiptKey[incarnation.target[root]]] \in visible \intersect stable
    /\ RecoveryReceiptRoots({IncarnationReceiptKey[incarnation.target[root]]}) \subseteq payloadsC
    /\ retainedReceipts' = retainedReceipts \union {IncarnationReceiptKey[incarnation.target[root]]}
    /\ UNCHANGED <<retentionLifecycleVars, DirectoryVars, directoryStages, directoryStageForeignChildren>>
\* Publication is distinct from selection/commit: an unselected immutable
\* target manifest already owns its exact staged identities and root closure.
IncarnationPublishPreparedReceipt(identity) ==
    /\ IncarnationPublishPrepared(identity)
    /\ RecoveryReceiptTarget[IncarnationReceiptKey[identity]] \in visible \intersect stable
    /\ RecoveryReceiptRoots({IncarnationReceiptKey[identity]}) \subseteq payloadsC
    /\ retainedReceipts' = retainedReceipts \union {IncarnationReceiptKey[identity]}
    /\ UNCHANGED <<retentionLifecycleVars, DirectoryVars, directoryStages, directoryStageForeignChildren>>
IncarnationPublishTargetReceipt(root) ==
    /\ IncarnationPublishTarget(root)
    /\ RecoveryReceiptTarget[IncarnationReceiptKey[incarnation.target[root]]] \in visible \intersect stable
    /\ RecoveryReceiptRoots({IncarnationReceiptKey[incarnation.target[root]]}) \subseteq payloadsC
    /\ retainedReceipts' = retainedReceipts \union {IncarnationReceiptKey[incarnation.target[root]]}
    /\ UNCHANGED <<retentionLifecycleVars, DirectoryVars, directoryStages, directoryStageForeignChildren>>
IncarnationEvolveReceiptKeys ==
    {incarnation.evolveReceipt} \union
      {IncarnationReceiptKey[identity] : identity \in IncarnationSnapshotIds(incarnation.evolveTarget)}
IncarnationPublishEvolveReceipts ==
    /\ IncarnationPublishEvolve
    /\ incarnation.evolveReceipt \notin {IncarnationReceiptKey[identity] : identity \in IncarnationIds}
    /\ RecoveryReceiptTarget[incarnation.evolveReceipt] \in visible \intersect stable
    /\ \A identity \in IncarnationSnapshotIds(incarnation.evolveTarget) :
          /\ RecoveryReceiptTarget[IncarnationReceiptKey[identity]] \in RecoveryReceiptLineage[incarnation.evolveReceipt]
          /\ RecoveryReceiptTarget[IncarnationReceiptKey[identity]] \in visible \intersect stable
    /\ RecoveryReceiptRoots(IncarnationEvolveReceiptKeys) \subseteq payloadsC
    /\ retainedReceipts' = retainedReceipts \union IncarnationEvolveReceiptKeys
    /\ UNCHANGED <<retentionLifecycleVars, DirectoryVars, directoryStages, directoryStageForeignChildren>>
IncarnationReceiptRegistryStep ==
    /\ UNCHANGED <<retentionLifecycleVars, incarnation>>
    /\ \E receipt \in RecoveryReceiptKeys :
          (RetainMigrationReceipt(receipt)
           /\ receipt \notin {IncarnationReceiptKey[identity] : identity \in IncarnationIds}
           /\ RecoveryReceiptRoots({receipt}) \subseteq payloadsC)
          \/ (ReleaseRecoveryReceipt(receipt) /\ RecoveryReceiptTarget[receipt] \notin visible \union stable)
IncarnationRecoveryVars == <<retentionLifecycleVars, RetainedDirectoryVars, incarnation>>
IncarnationReceiptsBound ==
    /\ retainedReceipts \subseteq RecoveryReceiptKeys
    /\ \A identity \in IncarnationIds : IncarnationReceiptKey[identity] \in retainedReceipts => identity \in incarnation.receipts
IncarnationRecoveryInit ==
    /\ RetentionLifecycleInit /\ DirectoryInit /\ IncarnationInit
    /\ directoryStages = [root \in DirectoryParents |-> <<"absent">>] /\ directoryStageForeignChildren = {}
    /\ retainedReceipts = RecoveryInitialReceipts \union
          {IncarnationReceiptKey[identity] : identity \in IncarnationInitialReceipts}
IncarnationRecoveryNext ==
    \/ RetentionLifecycleNext(TRUE) /\ UNCHANGED <<RetainedDirectoryVars, incarnation>>
       /\ IncarnationJournalAuthority /\ IncarnationProtectedGenerationAuthority
       /\ (\A root \in DirectoryParents : ~IncarnationRootNeedsSynchronization(root))
    \/ IncarnationNext /\ UNCHANGED <<retentionLifecycleVars, RetainedDirectoryVars>> /\ IncarnationDirectoryAuthority
    \/ \E root \in DirectoryParents :
          IncarnationRootJournalStep(root) \/ IncarnationCommitReceipt(root) \/ IncarnationPublishTargetReceipt(root)
    \/ \E destination \in Destinations : IncarnationClearTargetLeaf(destination)
    \/ IncarnationReceiptRegistryStep \/ IncarnationPublishEvolveReceipts
    \/ \E identity \in IncarnationIds : IncarnationPublishPreparedReceipt(identity)
IncarnationRecoverySpec == IncarnationRecoveryInit /\ [][IncarnationRecoveryNext]_IncarnationRecoveryVars
IncarnationRecoveryContract == GeneralRecoveryContract /\ IncarnationInvariant
    /\ GeneralHierarchyReceiptRootsPresent /\ IncarnationReceiptsBound

THEOREM IncarnationInitialReceiptSafety ==
  IncarnationRecoveryInit => GeneralHierarchyReceiptRootsPresent /\ IncarnationReceiptsBound
  BY SMT, IncarnationInitialAdmission, IncarnationCorrespondence, IncarnationParameters
  DEF IncarnationRecoveryInit, IncarnationInit, RetentionLifecycleInit,
      GeneralHierarchyReceiptRootsPresent, IncarnationReceiptsBound, IncarnationInitialPrepared

THEOREM IncarnationRecoveryProjectsLeafLifecycle == IncarnationRecoverySpec => RetentionLifecycleSpec
<1>1. IncarnationRecoveryInit => RetentionLifecycleInit
  BY DEF IncarnationRecoveryInit
<1>2. [IncarnationRecoveryNext]_IncarnationRecoveryVars => [RetentionLifecycleNext(TRUE)]_retentionLifecycleVars
  BY SMT DEF IncarnationRecoveryNext, IncarnationRecoveryVars, IncarnationRootJournalStep,
      IncarnationCommitReceipt, IncarnationClearTargetLeaf, IncarnationReceiptRegistryStep,
      IncarnationPublishTargetReceipt, IncarnationPublishEvolveReceipts, IncarnationPublishPreparedReceipt
<1>3. QED
  BY PTL, <1>1, <1>2 DEF IncarnationRecoverySpec, RetentionLifecycleSpec

THEOREM IncarnationRecoveryProjectsDirectoryDomain == IncarnationRecoverySpec => IncarnationSpec
<1>1. IncarnationRecoveryInit => IncarnationInit
  BY DEF IncarnationRecoveryInit
<1>2. [IncarnationRecoveryNext]_IncarnationRecoveryVars => [IncarnationNext]_IncarnationVars
  BY SMT, GeneralHierarchyDestinations
  DEF IncarnationRecoveryNext, IncarnationRecoveryVars, IncarnationVars, IncarnationNext,
      IncarnationRootJournalStep, IncarnationCommitReceipt, IncarnationClearTargetLeaf,
      IncarnationReceiptRegistryStep, IncarnationPublishTargetReceipt, IncarnationPublishEvolveReceipts,
      IncarnationPublishPreparedReceipt
<1>3. QED
  BY PTL, <1>1, <1>2 DEF IncarnationRecoverySpec, IncarnationSpec

THEOREM IncarnationReceiptRootStepSafety ==
  GeneralHierarchyReceiptRootsPresent /\ IncarnationRecoveryNext => GeneralHierarchyReceiptRootsPresent'
  BY SMT, GeneralHierarchyLifecyclePreservesReceiptRoots, IncarnationParameters
  DEF IncarnationRecoveryNext, IncarnationRootJournalStep, IncarnationClearTargetLeaf,
      IncarnationCommitReceipt, IncarnationReceiptRegistryStep, RetainMigrationReceipt,
      ReleaseRecoveryReceipt, GeneralHierarchyReceiptRootsPresent, RecoveryReceiptRoots,
      IncarnationPublishTargetReceipt, IncarnationPublishEvolveReceipts, IncarnationProtectedGenerationAuthority,
      IncarnationPublishPreparedReceipt,
      IncarnationEvolveReceiptKeys, IncarnationSnapshotIds,
      RetainedDirectoryVars, DirectoryStageVars, retentionLifecycleVars, payloadVars

THEOREM IncarnationReceiptRootSafety == IncarnationRecoverySpec => []GeneralHierarchyReceiptRootsPresent
<1>1. IncarnationRecoveryInit => GeneralHierarchyReceiptRootsPresent
  BY SMT, IncarnationInitialReceiptSafety
<1>2. GeneralHierarchyReceiptRootsPresent /\ [IncarnationRecoveryNext]_IncarnationRecoveryVars
       => GeneralHierarchyReceiptRootsPresent'
  BY IncarnationReceiptRootStepSafety DEF GeneralHierarchyReceiptRootsPresent, RecoveryReceiptRoots,
      IncarnationRecoveryVars, RetainedDirectoryVars, DirectoryStageVars, retentionLifecycleVars, payloadVars
<1>3. QED
  BY PTL, <1>1, <1>2 DEF IncarnationRecoverySpec

THEOREM IncarnationReceiptBindingStepSafety ==
  IncarnationInvariant /\ IncarnationReceiptsBound /\ IncarnationRecoveryNext => IncarnationReceiptsBound'
  BY SMT, IncarnationParameters, IncarnationReceiptUpdateFrame
  DEF IncarnationRecoveryNext, IncarnationReceiptsBound, IncarnationDirectoryAuthority,
      IncarnationRootJournalStep, IncarnationWriteEarlierRoot, IncarnationNormalizeRoot,
      IncarnationClearTargetLeaf, IncarnationSuspendLeaf, IncarnationCommitReceipt, IncarnationCommit,
      IncarnationReceiptRegistryStep, RetainMigrationReceipt, ReleaseRecoveryReceipt,
      IncarnationReleaseReceipt, IncarnationInvariant, IncarnationTypeOK, IncarnationEvidence,
      IncarnationPublishTargetReceipt, IncarnationPublishTarget, IncarnationPublishEvolveReceipts,
      IncarnationPublishPreparedReceipt, IncarnationPublishPrepared,
      IncarnationPublishEvolve, IncarnationEvolveReceiptKeys, IncarnationSnapshotIds,
      RetainedDirectoryVars, DirectoryStageVars

THEOREM IncarnationReceiptBindingSafety == IncarnationRecoverySpec => []IncarnationReceiptsBound
<1>1. IncarnationRecoveryInit => IncarnationReceiptsBound
  BY SMT, IncarnationInitialReceiptSafety
<1>2. IncarnationInvariant /\ IncarnationReceiptsBound /\ [IncarnationRecoveryNext]_IncarnationRecoveryVars
       => IncarnationReceiptsBound'
  BY IncarnationReceiptBindingStepSafety DEF IncarnationReceiptsBound, IncarnationRecoveryVars,
      RetainedDirectoryVars, DirectoryStageVars
<1>3. QED
  BY PTL, <1>1, <1>2, IncarnationRecoveryProjectsDirectoryDomain, ArbitraryFiniteIncarnationSafety
  DEF IncarnationRecoverySpec

THEOREM ArbitraryFiniteIncarnationRecoverySafety == IncarnationRecoverySpec => []IncarnationRecoveryContract
  BY PTL, IncarnationRecoveryProjectsLeafLifecycle, ArbitraryFiniteRepeatedRecoverySafety,
      IncarnationRecoveryProjectsDirectoryDomain, ArbitraryFiniteIncarnationSafety,
      IncarnationReceiptRootSafety, IncarnationReceiptBindingSafety
  DEF IncarnationRecoveryContract

THEOREM ExchangeLeafUndoWaitsForOriginalIncarnation ==
  ASSUME NEW destination \in Destinations, IncarnationJournalAuthority,
         RestoreStep(destination), IncarnationLeafChanges(destination),
         NEW root \in DirectoryAncestors[destination] \intersect incarnation.intents,
         incarnation.phase[root] = "undo", IncarnationBeforeOwned(root)
  PROVE incarnation.live[root] = incarnation.before[root]
  BY SMT DEF IncarnationJournalAuthority, IncarnationRestorationReady

THEOREM RootUndoIsSkippedAfterExactDirectoryRestoration ==
  ASSUME NEW root \in DirectoryParents, IncarnationJournalAuthority, IncarnationIsOwned(root)
  PROVE ~(RestoreStep(IncarnationRootDestination[root]) /\ IncarnationLeafChanges(IncarnationRootDestination[root]))
  BY SMT DEF IncarnationJournalAuthority, IncarnationIsWitness

THEOREM RootNormalizationUsesOrdinaryDurableUndo ==
  ASSUME NEW root \in DirectoryParents, IncarnationRootJournalStep(root), IncarnationNormalizeRoot(root),
         incarnation.phase[root] = "undo"
  PROVE /\ RestoreStep(IncarnationRootDestination[root])
        /\ cells'[IncarnationRootDestination[root]].durable.live.value = IncarnationObject[incarnation'.live[root]]
  BY SMT DEF IncarnationRootJournalStep, IncarnationWriteEarlierRoot, IncarnationRootCellCorrespondence

THEOREM IncarnationRetainedGenerationKeepsReceipt ==
  ASSUME NEW receipt \in retainedReceipts, RecoveryReceiptTarget[receipt] \in visible \union stable,
         IncarnationRecoveryNext
  PROVE receipt \in retainedReceipts'
  BY SMT DEF IncarnationRecoveryNext, IncarnationRootJournalStep, IncarnationCommitReceipt,
      IncarnationClearTargetLeaf, IncarnationReceiptRegistryStep, RetainMigrationReceipt,
      ReleaseRecoveryReceipt, IncarnationPublishTargetReceipt, IncarnationPublishEvolveReceipts,
      IncarnationPublishPreparedReceipt,
      RetainedDirectoryVars, DirectoryStageVars

THEOREM ReceiptPredecessorGenerationPruningIsForbidden ==
  ASSUME GeneralHierarchyCollectionAuthority, NEW receipt \in retainedReceipts,
         NEW predecessor \in RecoveryReceiptLineage[receipt] \ {RecoveryReceiptTarget[receipt]}
  PROVE ~RemoveCollectedGeneration(predecessor)
  BY SMT DEF GeneralHierarchyCollectionAuthority

THEOREM CurrentDeclarationGrantsNewLeafMutation ==
  ASSUME NEW destination \in Destinations, IncarnationJournalAuthority,
         WriterStep(destination), IncarnationLeafChanges(destination),
         cells'[destination].cached.live.value # Absent
  PROVE destination \in incarnation.declared
  BY SMT DEF IncarnationJournalAuthority

THEOREM BorrowedRootCannotBeFlatLeafMutation ==
  ASSUME NEW root \in DirectoryParents, IncarnationJournalAuthority,
         incarnation.live[root] \in IncarnationBorrowedImages
  PROVE ~((WriterStep(IncarnationRootDestination[root]) \/ RestoreStep(IncarnationRootDestination[root]))
           /\ IncarnationLeafChanges(IncarnationRootDestination[root]))
  BY SMT DEF IncarnationJournalAuthority, IncarnationIsWitness

THEOREM EvolveLeafUndoWaitsForOriginalShape ==
  ASSUME NEW destination \in Destinations, IncarnationJournalAuthority,
         RestoreStep(destination), IncarnationLeafChanges(destination),
         incarnation.evolveActive, incarnation.evolveUndo,
         NEW root \in DirectoryAncestors[destination],
         incarnation.evolveOriginal[root] \in IncarnationOwnedImages \union IncarnationBorrowedImages
  PROVE incarnation.live[root] = incarnation.evolveOriginal[root]
  BY SMT DEF IncarnationJournalAuthority, IncarnationRestorationReady

THEOREM PublishedUnselectedEvolveBindsReceipts ==
  ASSUME IncarnationInvariant, IncarnationPublishEvolveReceipts
  PROVE /\ IncarnationSnapshotIds(incarnation.evolveTarget) \subseteq incarnation'.receipts
        /\ incarnation.evolveReceipt \in retainedReceipts'
  BY SMT DEF IncarnationPublishEvolveReceipts, IncarnationPublishEvolve, IncarnationEvolveReceiptKeys,
      IncarnationInvariant, IncarnationTypeOK

THEOREM PublishedIdentityCannotBeDiscardedWhileReceiptRetained ==
  ASSUME IncarnationReceiptsBound, NEW identity \in IncarnationIds,
         IncarnationReceiptKey[identity] \in retainedReceipts
  PROVE ~IncarnationDiscardStage(identity)
  BY SMT DEF IncarnationReceiptsBound, IncarnationDiscardStage

THEOREM PendingEvolvePublicationProtectsItsGeneration ==
  ASSUME IncarnationProtectedGenerationAuthority, incarnation.evolveActive, incarnation.evolvePublished
  PROVE ~RemoveCollectedGeneration(RecoveryReceiptTarget[incarnation.evolveReceipt])
  BY SMT DEF IncarnationProtectedGenerationAuthority

THEOREM PreparedPublicationBeforeIntentPreservesIdentity ==
  ASSUME IncarnationInvariant, NEW identity \in IncarnationIds, incarnation.intents = {},
         ~incarnation.evolveActive, IncarnationPublishPreparedReceipt(identity)
  PROVE /\ incarnation'.intents = {}
        /\ identity \in incarnation'.receipts
        /\ IncarnationReceiptKey[identity] \in retainedReceipts'
  BY SMT DEF IncarnationPublishPreparedReceipt, IncarnationPublishPrepared,
      IncarnationInvariant, IncarnationTypeOK

THEOREM EvolveNodeBirthWaitsForOrdinaryLeafAbsence ==
  ASSUME NEW root \in DirectoryParents, IncarnationDirectoryAuthority,
         IncarnationEvolveInstallNode(root) \/ IncarnationEvolveRestoreNode(root)
  PROVE GeneralHierarchyLeafAbsent(IncarnationRootDestination[root])
  BY SMT DEF IncarnationDirectoryAuthority

=============================================================================
