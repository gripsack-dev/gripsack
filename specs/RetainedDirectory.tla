---- MODULE RetainedDirectory ----
EXTENDS OwnedDirectory

\* Immutable receipt admission supplies the complete predecessor-generation
\* closure. The closure includes the receipt's own target, not only its parent.
\* This is an input-graph admission boundary, not an assumption of GC safety.
CONSTANTS DirectoryGenerations, DirectoryPayloads, RecoveryReceiptKeys, RecoveryReceiptTarget,
          DirectoryGenerationPredecessors, RecoveryReceiptLineage, DirectoryGenerationRoots,
          DirectoryEarlierRoot
DirectoryRootImages == {<<"absent">>} \union
    {<<kind, target>> : kind \in {"symlink", "file"}, target \in DirectoryTargets}
ASSUME DirectoryReceiptParameters ==
    /\ IsFiniteSet(RecoveryReceiptKeys) /\ DirectoryParents \subseteq RecoveryReceiptKeys
    /\ RecoveryReceiptTarget \in [RecoveryReceiptKeys -> DirectoryGenerations]
    /\ DirectoryGenerationPredecessors \in [DirectoryGenerations -> SUBSET DirectoryGenerations]
    /\ RecoveryReceiptLineage \in [RecoveryReceiptKeys -> SUBSET DirectoryGenerations]
    /\ DirectoryGenerationRoots \in [DirectoryGenerations -> SUBSET DirectoryPayloads]
    /\ DirectoryEarlierRoot \in [DirectoryParents -> DirectoryRootImages]
    /\ \A receipt \in RecoveryReceiptKeys :
          /\ RecoveryReceiptTarget[receipt] \in RecoveryReceiptLineage[receipt]
          /\ \A generation \in RecoveryReceiptLineage[receipt] :
                DirectoryGenerationPredecessors[generation] \subseteq RecoveryReceiptLineage[receipt]

VARIABLES directoryStages, retainedReceipts, directoryStageForeignChildren
DirectoryStageVars == <<directoryStages, retainedReceipts, directoryStageForeignChildren>>
RetainedDirectoryVars == <<DirectoryVars, DirectoryStageVars>>
RecoveryReceiptRoots(receipts) == UNION {DirectoryGenerationRoots[generation] : generation \in
    UNION {RecoveryReceiptLineage[receipt] : receipt \in receipts}}
RetainedDirectoryInit ==
    /\ DirectoryInit
    /\ directoryStages = [parent \in DirectoryParents |-> <<"absent">>]
    /\ retainedReceipts = {} /\ directoryStageForeignChildren = {}
DirectoryRetainReceipt(parent) ==
    /\ parent \in directoryIntents /\ directoryLive[parent] = DirectoryOwned(parent)
    /\ retainedReceipts' = retainedReceipts \union {parent}
    /\ UNCHANGED <<DirectoryVars, directoryStages, directoryStageForeignChildren>>
\* Dropping the final retained reference is a generation-retention decision;
\* it grants no namespace deletion until exact empty-stage admission below.
ReleaseRecoveryReceipt(receipt) ==
    /\ retainedReceipts' = retainedReceipts \ {receipt}
    /\ UNCHANGED <<DirectoryVars, directoryStages, directoryStageForeignChildren>>
\* Direct migration manifests carry the same immutable predecessor closure
\* without requiring any directory transition. Ordinary history/hook actions
\* cannot release either receipt family; only retention releases the last key.
RetainMigrationReceipt(receipt) ==
    /\ receipt \notin DirectoryParents /\ DirectoryJournalVersion = 3
    /\ retainedReceipts' = retainedReceipts \union {receipt}
    /\ UNCHANGED <<DirectoryVars, directoryStages, directoryStageForeignChildren>>
\* Reverse removal releases a physically absent child while its ordinary undo
\* record remains live. Its binding is retained for parent-first recovery.
DirectorySuspendLeaf(leaf) ==
    /\ leaf \in directoryLeaves /\ DirectoryBindingMatches(leaf)
    /\ directoryLeaves' = directoryLeaves \ {leaf}
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryLive, directoryPending,
                   directoryBindings, directoryForeignChildren, directoryRetired,
                   directoryViolations, DirectoryStageVars>>
DirectoryStageOwned(parent) ==
    /\ parent \in directoryPending /\ directoryLive[parent] = DirectoryOwned(parent)
    /\ directoryStages[parent] = <<"absent">>
    /\ DirectoryAncestorsMatch(parent) /\ DirectoryChildrenSettled(parent)
    /\ parent \notin directoryForeignChildren
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = <<"absent">>]
    /\ directoryStages' = [directoryStages EXCEPT ![parent] = DirectoryOwned(parent)]
    /\ directoryRetired' = directoryRetired \union {parent}
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryViolations,
                   retainedReceipts, directoryStageForeignChildren>>
DirectorySealReverseIntent(parent) ==
    /\ DirectoryJournalVersion = 3 /\ parent \in directoryPrepared
    /\ directoryStages[parent] = DirectoryOwned(parent)
    /\ directoryIntents' = directoryIntents \union {parent}
    /\ UNCHANGED <<directoryPrepared, directoryLive, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryRetired,
                   directoryViolations, DirectoryStageVars>>
\* Root leaf writes may select a still-earlier lineage after the exact owned
\* directory has been staged. Their acknowledged durable effects are joined
\* to the ordinary leaf kernel in GeneralRecoveryProofs, never guessed from a
\* directory enum. Recovery normalizes this root before reinstalling children.
DirectoryWriteEarlierRoot(parent) ==
    /\ parent \in directoryIntents /\ parent \notin directoryPending
    /\ directoryStages[parent] = DirectoryOwned(parent)
    /\ directoryLive[parent] = DirectoryPrior(parent) /\ DirectoryAncestorsMatch(parent)
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = DirectoryEarlierRoot[parent]]
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryRetired,
                   directoryViolations, DirectoryStageVars>>
DirectoryNormalizeRoot(parent) ==
    /\ parent \in directoryIntents /\ parent \notin directoryPending
    /\ directoryStages[parent] = DirectoryOwned(parent)
    /\ directoryLive[parent] = DirectoryEarlierRoot[parent] /\ DirectoryAncestorsMatch(parent)
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = DirectoryPrior(parent)]
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryRetired,
                   directoryViolations, DirectoryStageVars>>
\* An uncommitted Reverse uses its still-durable intent; a later retained-target
\* reactivation first seals a fresh intent. Both reinstall the parent before
\* ordinary leaf restoration can acquire its ancestor-bound authority.
\* This lower slice deliberately covers EMPTY-stage installation only.
\* Original-container compensation with foreign additions is represented by
\* IncarnationRestoreOriginal / IncarnationEvolveRestoreNode: they preserve
\* those contents while moving the exact original inode, without leaf authority.
DirectoryReverseUnlink(parent) ==
    /\ parent \in directoryIntents /\ directoryStages[parent] = DirectoryOwned(parent)
    /\ directoryLive[parent] = DirectoryPrior(parent) /\ DirectoryAncestorsMatch(parent)
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = <<"absent">>]
    /\ directoryPending' = directoryPending \union {parent}
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryLeaves, directoryBindings,
                   directoryForeignChildren, directoryRetired, directoryViolations, DirectoryStageVars>>
DirectoryReinstallStage(parent) ==
    /\ parent \in directoryPending /\ parent \in directoryIntents
    /\ directoryLive[parent] = <<"absent">> /\ directoryStages[parent] = DirectoryOwned(parent)
    /\ DirectoryAncestorsMatch(parent) /\ parent \notin directoryStageForeignChildren
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = DirectoryOwned(parent)]
    /\ directoryStages' = [directoryStages EXCEPT ![parent] = <<"absent">>]
    /\ directoryRetired' = directoryRetired \ {parent}
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryViolations,
                   retainedReceipts, directoryStageForeignChildren>>
DirectoryDiscardStage(parent) ==
    /\ parent \notin retainedReceipts /\ parent \notin directoryPending
    /\ parent \notin directoryIntents /\ parent \notin directoryStageForeignChildren
    /\ directoryStages[parent] = DirectoryOwned(parent)
    /\ directoryStages' = [directoryStages EXCEPT ![parent] = <<"absent">>]
    /\ UNCHANGED <<DirectoryVars, retainedReceipts, directoryStageForeignChildren>>
DirectorySubstituteStage(parent) ==
    /\ directoryStages' = [directoryStages EXCEPT ![parent] = DirectoryForeign(parent)]
    /\ UNCHANGED <<DirectoryVars, retainedReceipts, directoryStageForeignChildren>>
DirectoryAddForeignStageChild(parent) ==
    /\ directoryStages[parent] = DirectoryOwned(parent)
    /\ directoryStageForeignChildren' = directoryStageForeignChildren \union {parent}
    /\ UNCHANGED <<DirectoryVars, directoryStages, retainedReceipts>>
RetainedDirectoryNext ==
    \/ DirectoryNext /\ UNCHANGED DirectoryStageVars
       /\ (\A parent \in retainedReceipts \intersect DirectoryParents : ~DirectoryRemove(parent))
    \/ \E parent \in DirectoryParents : DirectoryRetainReceipt(parent)
          \/ DirectoryStageOwned(parent) \/ DirectorySealReverseIntent(parent)
          \/ DirectoryReverseUnlink(parent) \/ DirectoryReinstallStage(parent)
          \/ DirectoryDiscardStage(parent) \/ DirectorySubstituteStage(parent)
          \/ DirectoryAddForeignStageChild(parent)
          \/ DirectoryWriteEarlierRoot(parent) \/ DirectoryNormalizeRoot(parent)
    \/ \E leaf \in DirectoryLeaves : DirectorySuspendLeaf(leaf)
    \/ \E receipt \in RecoveryReceiptKeys : RetainMigrationReceipt(receipt) \/ ReleaseRecoveryReceipt(receipt)
RetainedDirectorySpec == RetainedDirectoryInit /\ [][RetainedDirectoryNext]_RetainedDirectoryVars
RetainedDirectoryInvariant ==
    /\ DirectoryInvariant
    /\ directoryStages \in [DirectoryParents -> DirectoryImages]
    /\ retainedReceipts \subseteq RecoveryReceiptKeys
    /\ directoryStageForeignChildren \subseteq DirectoryParents
=============================================================================
