---- MODULE OwnedDirectoryProofs ----
EXTENDS OwnedDirectory, TLAPS

ASSUME DirectoryCorrectProtocol == DirectoryFault = "none"

THEOREM DirectoryInitialSafety == DirectoryInit => DirectoryInvariant
  BY SMT, DirectoryParameters
  DEF DirectoryInit, DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence,
      DirectorySafety, DirectoryPrior, DirectoryOwned, DirectoryImages, DirectoryViolations

THEOREM DirectoryMatchingAncestorsHaveEvidence ==
  ASSUME DirectoryInvariant, NEW node \in DirectoryParents \union DirectoryLeaves,
         DirectoryAncestorsMatch(node)
  PROVE DirectoryAncestors[node] \subseteq directoryPending
  BY SMT, DirectoryParameters
  DEF DirectoryInvariant, DirectoryEvidence, DirectoryAncestorsMatch

THEOREM DirectoryPreparationSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryPrepare(parent)
  PROVE DirectoryInvariant'
  BY SMT DEF DirectoryPrepare, DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety

THEOREM DirectoryIntentSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectorySealIntent(parent)
  PROVE DirectoryInvariant'
  BY SMT DEF DirectorySealIntent, DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety

THEOREM DirectoryUnlinkSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryUnlinkPrior(parent)
  PROVE DirectoryInvariant'
  BY SMT, DirectoryParameters, DirectoryCorrectProtocol, DirectoryMatchingAncestorsHaveEvidence
  DEF DirectoryUnlinkPrior, DirectoryAncestorGuard, DirectoryAncestorsMatch,
      DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety,
      DirectoryImages, DirectoryOwned, DirectoryRecordViolation

THEOREM DirectoryInstallSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryInstall(parent)
  PROVE DirectoryInvariant'
  BY SMT, DirectoryParameters, DirectoryCorrectProtocol
  DEF DirectoryInstall, DirectoryAncestorGuard, DirectoryInvariant, DirectoryTypeOK,
      DirectoryEvidence, DirectorySafety, DirectoryOwned, DirectoryImages, DirectoryRecordViolation

THEOREM DirectoryLeafExposureSafety ==
  ASSUME DirectoryInvariant, NEW leaf \in DirectoryLeaves, DirectoryExposeLeaf(leaf)
  PROVE DirectoryInvariant'
  BY SMT, DirectoryParameters, DirectoryCorrectProtocol, DirectoryMatchingAncestorsHaveEvidence
  DEF DirectoryExposeLeaf, DirectoryAncestorGuard, DirectoryInvariant, DirectoryTypeOK,
      DirectoryEvidence, DirectorySafety, DirectoryRecordViolation

THEOREM DirectoryLeafReleaseSafety ==
  ASSUME DirectoryInvariant, NEW leaf \in DirectoryLeaves, DirectoryReleaseLeaf(leaf)
  PROVE DirectoryInvariant'
  BY SMT, DirectoryCorrectProtocol
  DEF DirectoryReleaseLeaf, DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence,
      DirectorySafety, DirectoryRecordViolation

THEOREM DirectoryRemovalSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryRemove(parent)
  PROVE DirectoryInvariant'
  BY SMT, DirectoryParameters, DirectoryCorrectProtocol
  DEF DirectoryRemove, DirectoryAncestorGuard, DirectoryChildGuard,
      DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety,
      DirectoryOwned, DirectoryImages, DirectoryRecordViolation

THEOREM DirectoryLinkRestorationSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryRestoreLink(parent)
  PROVE DirectoryInvariant'
  BY SMT, DirectoryParameters, DirectoryCorrectProtocol
  DEF DirectoryRestoreLink, DirectoryAncestorGuard, DirectoryChildGuard, DirectoryChildrenSettled,
      DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety,
      DirectoryPrior, DirectoryOwned, DirectoryImages, DirectoryRecordViolation

THEOREM DirectoryIntentCleanupSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryClearIntent(parent)
  PROVE DirectoryInvariant'
  BY SMT DEF DirectoryClearIntent, DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety

THEOREM DirectoryForeignSubstitutionSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectorySubstitute(parent)
  PROVE DirectoryInvariant'
  BY SMT, DirectoryParameters
  DEF DirectorySubstitute, DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence,
      DirectorySafety, DirectoryForeign, DirectoryOwned, DirectoryImages

THEOREM DirectoryForeignChildSafety ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryAddForeignChild(parent)
  PROVE DirectoryInvariant'
  BY SMT DEF DirectoryAddForeignChild, DirectoryInvariant, DirectoryTypeOK, DirectoryEvidence, DirectorySafety

THEOREM DirectoryInduction == DirectoryInvariant /\ DirectoryNext => DirectoryInvariant'
  BY SMT, DirectoryPreparationSafety, DirectoryIntentSafety, DirectoryUnlinkSafety,
      DirectoryInstallSafety, DirectoryLeafExposureSafety, DirectoryLeafReleaseSafety,
      DirectoryRemovalSafety, DirectoryLinkRestorationSafety, DirectoryIntentCleanupSafety,
      DirectoryForeignSubstitutionSafety, DirectoryForeignChildSafety
  DEF DirectoryNext, DirectoryCrash, DirectoryVars, DirectoryInvariant,
      DirectoryTypeOK, DirectoryEvidence, DirectorySafety

THEOREM ArbitraryFiniteDirectorySafety == DirectorySpec => []DirectoryInvariant
<1>1. DirectoryInit => DirectoryInvariant
  BY DirectoryInitialSafety
<1>2. DirectoryInvariant /\ [DirectoryNext]_DirectoryVars => DirectoryInvariant'
  BY DirectoryInduction DEF DirectoryVars, DirectoryInvariant, DirectoryTypeOK,
      DirectoryEvidence, DirectorySafety
<1>3. QED
  BY PTL, <1>1, <1>2 DEF DirectorySpec

THEOREM DirectoryMutationRequiresDurableIntent ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryUnlinkPrior(parent)
  PROVE /\ parent \in directoryPrepared /\ parent \in directoryIntents
        /\ directoryLive[parent] = DirectoryPrior(parent)
  BY SMT, DirectoryCorrectProtocol DEF DirectoryUnlinkPrior

THEOREM DirectoryCleanupIsChildBeforeParent ==
  ASSUME NEW parent \in DirectoryParents,
         DirectoryRemove(parent) \/ DirectoryRestoreLink(parent)
  PROVE DirectoryChildrenSettled(parent)
  BY SMT, DirectoryCorrectProtocol DEF DirectoryRemove, DirectoryRestoreLink, DirectoryChildGuard

THEOREM DirectoryCleanupPreservesForeignObjects ==
  ASSUME NEW parent \in DirectoryParents, DirectoryRemove(parent)
  PROVE directoryLive[parent] = DirectoryOwned(parent) /\ parent \notin directoryForeignChildren
  BY DEF DirectoryRemove

THEOREM DirectoryRestorationUsesExactPredecessor ==
  ASSUME DirectoryInvariant, NEW parent \in DirectoryParents, DirectoryRestoreLink(parent)
  PROVE directoryLive'[parent] = DirectoryPrior(parent)
  BY SMT DEF DirectoryRestoreLink, DirectoryInvariant, DirectoryTypeOK

THEOREM DirectoryLeafRecoveryBindsAncestors ==
  ASSUME NEW leaf \in DirectoryLeaves, DirectoryReleaseLeaf(leaf)
  PROVE DirectoryBindingMatches(leaf)
  BY SMT, DirectoryCorrectProtocol DEF DirectoryReleaseLeaf

THEOREM DirectoryForeignConflictRetainsIntent ==
  ASSUME DirectoryInvariant, NEW parent \in directoryPending,
         directoryLive[parent] = DirectoryForeign(parent) \/ parent \in directoryForeignChildren,
         DirectoryNext
  PROVE parent \in directoryIntents'
  BY SMT, DirectoryParameters
  DEF DirectoryInvariant, DirectoryEvidence, DirectoryTypeOK, DirectoryNext,
      DirectoryPrepare, DirectorySealIntent, DirectoryUnlinkPrior, DirectoryInstall,
      DirectoryExposeLeaf, DirectoryReleaseLeaf, DirectoryRemove, DirectoryRestoreLink,
      DirectoryClearIntent, DirectorySubstitute, DirectoryAddForeignChild,
      DirectoryCrash, DirectoryVars, DirectoryForeign, DirectoryOwned, DirectoryPrior

THEOREM LegacyJournalCannotAuthorizeDirectoryMutation ==
  ASSUME DirectoryInvariant, DirectoryJournalVersion \in {1, 2}, NEW parent \in DirectoryParents
  PROVE ~DirectoryUnlinkPrior(parent) /\ ~DirectoryInstall(parent) /\ ~DirectorySealIntent(parent)
  BY SMT, DirectoryCorrectProtocol
  DEF DirectoryInvariant, DirectoryEvidence, DirectoryUnlinkPrior, DirectoryInstall, DirectorySealIntent

=============================================================================
