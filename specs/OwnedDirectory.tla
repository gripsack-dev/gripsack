---- MODULE OwnedDirectory ----
EXTENDS Naturals, FiniteSets, Sequences

\* A directory-domain authority alongside, not in place of, the leaf journal.
\* Each transition below acknowledges a durable filesystem effect. The finite
\* DirectoryHierarchy explorer additionally separates cached writes, barriers,
\* and power loss. This theorem seam does not infer durability from visibility.
\* Identity equality is opaque but exact: admitted runtime identities contain
\* filesystem kind, fstatvfs FSID, inode, and required statx birth seconds/nanos,
\* not st_dev or mount ID. Missing FSID/birth time refuses admission. The model
\* assumes the filesystem-qualified identity contract; it does not establish
\* filesystem-specific FSID semantics or reboot/remount/hardware qualification.
CONSTANTS DirectoryParents, DirectoryLinkParents, DirectoryLeaves, DirectoryAncestors,
          DirectoryIdentities, DirectoryTargets, DirectoryPreparedIdentity,
          DirectoryForeignIdentity, DirectoryOriginalTarget, DirectoryFault, DirectoryJournalVersion
ASSUME DirectoryParameters ==
    /\ IsFiniteSet(DirectoryParents) /\ IsFiniteSet(DirectoryLeaves)
    /\ DirectoryJournalVersion \in {1, 2, 3}
    /\ DirectoryParents \intersect DirectoryLeaves = {}
    /\ DirectoryLinkParents \subseteq DirectoryParents
    /\ DirectoryAncestors \in [DirectoryParents \union DirectoryLeaves -> SUBSET DirectoryParents]
    /\ \A node \in DirectoryParents \union DirectoryLeaves :
          /\ node \notin DirectoryAncestors[node]
          /\ \A parent \in DirectoryAncestors[node] :
                DirectoryAncestors[parent] \subseteq DirectoryAncestors[node]
    /\ DirectoryPreparedIdentity \in [DirectoryParents -> DirectoryIdentities]
    /\ DirectoryForeignIdentity \in [DirectoryParents -> DirectoryIdentities]
    /\ DirectoryOriginalTarget \in [DirectoryParents -> DirectoryTargets]
    /\ \A parent \in DirectoryParents :
          DirectoryPreparedIdentity[parent] # DirectoryForeignIdentity[parent]
    /\ \A first, second \in DirectoryParents :
          DirectoryPreparedIdentity[first] = DirectoryPreparedIdentity[second] => first = second
    /\ DirectoryFault \in {"none", "missing_intent_barrier", "parent_before_child", "ancestor_substitution"}

\* Descendant directories may have an absent predecessor; only converted
\* symlink parents restore a link. A restored descendant link is not empty.
DirectoryPrior(parent) ==
    IF parent \in DirectoryLinkParents THEN <<"symlink", DirectoryOriginalTarget[parent]>> ELSE <<"absent">>
DirectoryOwned(parent) == <<"directory", DirectoryPreparedIdentity[parent]>>
DirectoryForeign(parent) == <<"directory", DirectoryForeignIdentity[parent]>>
DirectoryImages == {<<"absent">>} \union
    {<<"symlink", target>> : target \in DirectoryTargets} \union
    {<<"file", target>> : target \in DirectoryTargets} \union
    {<<"directory", identity>> : identity \in DirectoryIdentities}
DirectoryViolations == {"barrier", "ancestor", "order", "foreign"}

VARIABLES directoryPrepared, directoryIntents, directoryLive, directoryPending,
          directoryLeaves, directoryBindings, directoryForeignChildren,
          directoryRetired, directoryViolations
DirectoryVars == <<directoryPrepared, directoryIntents, directoryLive, directoryPending,
    directoryLeaves, directoryBindings, directoryForeignChildren, directoryRetired, directoryViolations>>

DirectoryAncestorsMatch(node) ==
    \A parent \in DirectoryAncestors[node] : directoryLive[parent] = DirectoryOwned(parent)
DirectoryBindingMatches(leaf) ==
    \A parent \in DirectoryAncestors[leaf] :
        directoryLive[parent] = <<"directory", directoryBindings[leaf][parent]>>
DirectoryChildrenSettled(parent) ==
    /\ \A leaf \in directoryLeaves : parent \notin DirectoryAncestors[leaf]
    /\ \A child \in directoryPending : parent \notin DirectoryAncestors[child]
    /\ \A child \in DirectoryParents :
          parent \in DirectoryAncestors[child] => directoryLive[child] = <<"absent">>
DirectoryAncestorGuard(node) == DirectoryFault = "ancestor_substitution" \/ DirectoryAncestorsMatch(node)
DirectoryChildGuard(parent) == DirectoryFault = "parent_before_child" \/ DirectoryChildrenSettled(parent)
DirectoryRecordViolation(condition, name) == IF condition THEN {} ELSE {name}

DirectoryInit ==
    /\ directoryPrepared = {} /\ directoryIntents = {} /\ directoryPending = {}
    /\ directoryLive = [parent \in DirectoryParents |-> DirectoryPrior(parent)]
    /\ directoryLeaves = {}
    /\ directoryBindings = [leaf \in DirectoryLeaves |-> DirectoryPreparedIdentity]
    /\ directoryForeignChildren = {} /\ directoryRetired = {} /\ directoryViolations = {}

\* Exclusive preparation has completed, including persistence of its exact
\* identity. Old journal versions have no action granting this authority.
DirectoryPrepare(parent) ==
    /\ parent \notin directoryPrepared \union directoryRetired
    /\ directoryPrepared' = directoryPrepared \union {parent}
    /\ UNCHANGED <<directoryIntents, directoryLive, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryRetired, directoryViolations>>
DirectorySealIntent(parent) ==
    /\ DirectoryJournalVersion = 3
    /\ parent \in directoryPrepared /\ parent \notin directoryRetired
    /\ directoryIntents' = directoryIntents \union {parent}
    /\ UNCHANGED <<directoryPrepared, directoryLive, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryRetired, directoryViolations>>
DirectoryUnlinkPrior(parent) ==
    /\ parent \in directoryPrepared /\ parent \notin directoryRetired
    /\ DirectoryFault = "missing_intent_barrier" \/ parent \in directoryIntents
    /\ directoryLive[parent] = DirectoryPrior(parent)
    /\ DirectoryAncestorGuard(parent)
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = <<"absent">>]
    /\ directoryPending' = directoryPending \union {parent}
    /\ directoryViolations' = directoryViolations
          \union DirectoryRecordViolation(parent \in directoryIntents, "barrier")
          \union DirectoryRecordViolation(DirectoryAncestorsMatch(parent), "ancestor")
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryLeaves, directoryBindings,
                   directoryForeignChildren, directoryRetired>>
DirectoryInstall(parent) ==
    /\ parent \in directoryPending /\ directoryLive[parent] = <<"absent">>
    /\ parent \notin directoryRetired
    /\ DirectoryAncestorGuard(parent)
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = DirectoryOwned(parent)]
    /\ directoryViolations' = directoryViolations
          \union DirectoryRecordViolation(DirectoryAncestorsMatch(parent), "ancestor")
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryRetired>>

\* A leaf exposure is an authority handshake, not a replacement for UndoCell.
\* Capture the complete ancestor identities before the ordinary leaf write;
\* release it only after that leaf's restoration AND journal cleanup are stable.
DirectoryExposeLeaf(leaf) ==
    /\ DirectoryAncestorGuard(leaf)
    /\ directoryLeaves' = directoryLeaves \union {leaf}
    /\ directoryBindings' = [directoryBindings EXCEPT ![leaf] = DirectoryPreparedIdentity]
    /\ directoryViolations' = directoryViolations
          \union DirectoryRecordViolation(DirectoryAncestorsMatch(leaf), "ancestor")
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryLive, directoryPending,
                   directoryForeignChildren, directoryRetired>>
DirectoryReleaseLeaf(leaf) ==
    /\ leaf \in directoryLeaves
    /\ DirectoryFault = "ancestor_substitution" \/ DirectoryBindingMatches(leaf)
    /\ directoryLeaves' = directoryLeaves \ {leaf}
    /\ directoryViolations' = directoryViolations
          \union DirectoryRecordViolation(DirectoryBindingMatches(leaf), "ancestor")
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryLive, directoryPending,
                   directoryBindings, directoryForeignChildren, directoryRetired>>
DirectoryRemove(parent) ==
    /\ parent \in directoryPending /\ directoryLive[parent] = DirectoryOwned(parent)
    /\ DirectoryAncestorGuard(parent) /\ DirectoryChildGuard(parent)
    /\ parent \notin directoryForeignChildren
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = <<"absent">>]
    /\ directoryRetired' = directoryRetired \union {parent}
    /\ directoryViolations' = directoryViolations
          \union DirectoryRecordViolation(DirectoryAncestorsMatch(parent), "ancestor")
          \union DirectoryRecordViolation(DirectoryChildrenSettled(parent), "order")
          \union DirectoryRecordViolation(directoryLive[parent] = DirectoryOwned(parent)
                    /\ parent \notin directoryForeignChildren, "foreign")
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren>>
DirectoryRestoreLink(parent) ==
    /\ parent \in directoryPending /\ directoryLive[parent] = <<"absent">>
    /\ DirectoryAncestorGuard(parent) /\ DirectoryChildGuard(parent)
    /\ parent \notin directoryForeignChildren
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = DirectoryPrior(parent)]
    /\ directoryPending' = directoryPending \ {parent}
    /\ directoryRetired' = directoryRetired \union {parent}
    /\ directoryViolations' = directoryViolations
          \union DirectoryRecordViolation(DirectoryAncestorsMatch(parent), "ancestor")
          \union DirectoryRecordViolation(DirectoryChildrenSettled(parent), "order")
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryLeaves, directoryBindings,
                   directoryForeignChildren>>
DirectoryClearIntent(parent) ==
    /\ parent \in directoryIntents /\ parent \notin directoryPending
    /\ directoryLive[parent] = DirectoryPrior(parent)
    /\ DirectoryChildrenSettled(parent) /\ parent \notin directoryForeignChildren
    /\ directoryIntents' = directoryIntents \ {parent}
    /\ UNCHANGED <<directoryPrepared, directoryLive, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryRetired, directoryViolations>>

\* Outside actors may substitute a same-kind directory or add an unknown child.
\* No operator transition removes either; evidence survives the conflict.
DirectorySubstitute(parent) ==
    /\ directoryLive' = [directoryLive EXCEPT ![parent] = DirectoryForeign(parent)]
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryPending, directoryLeaves,
                   directoryBindings, directoryForeignChildren, directoryRetired, directoryViolations>>
DirectoryAddForeignChild(parent) ==
    /\ directoryLive[parent] = DirectoryOwned(parent)
    /\ directoryForeignChildren' = directoryForeignChildren \union {parent}
    /\ UNCHANGED <<directoryPrepared, directoryIntents, directoryLive, directoryPending,
                   directoryLeaves, directoryBindings, directoryRetired, directoryViolations>>
DirectoryCrash == UNCHANGED DirectoryVars
DirectoryNext ==
    \/ \E parent \in DirectoryParents : DirectoryPrepare(parent) \/ DirectorySealIntent(parent)
          \/ DirectoryUnlinkPrior(parent) \/ DirectoryInstall(parent) \/ DirectoryRemove(parent)
          \/ DirectoryRestoreLink(parent) \/ DirectoryClearIntent(parent)
          \/ DirectorySubstitute(parent) \/ DirectoryAddForeignChild(parent)
    \/ \E leaf \in DirectoryLeaves : DirectoryExposeLeaf(leaf) \/ DirectoryReleaseLeaf(leaf)
    \/ DirectoryCrash
DirectorySpec == DirectoryInit /\ [][DirectoryNext]_DirectoryVars

DirectoryTypeOK ==
    /\ directoryPrepared \subseteq DirectoryParents /\ directoryIntents \subseteq DirectoryParents
    /\ directoryPending \subseteq DirectoryParents /\ directoryRetired \subseteq DirectoryParents
    /\ directoryLive \in [DirectoryParents -> DirectoryImages]
    /\ directoryLeaves \subseteq DirectoryLeaves
    /\ directoryBindings \in [DirectoryLeaves -> [DirectoryParents -> DirectoryIdentities]]
    /\ directoryForeignChildren \subseteq DirectoryParents
    /\ directoryViolations \subseteq DirectoryViolations
DirectoryEvidence ==
    /\ directoryIntents \subseteq directoryPrepared
    /\ (directoryIntents # {} => DirectoryJournalVersion = 3)
    /\ directoryPending \subseteq directoryIntents
    /\ \A parent \in DirectoryParents :
          directoryLive[parent] = DirectoryOwned(parent) => parent \in directoryPending
    /\ \A leaf \in directoryLeaves : DirectoryAncestors[leaf] \subseteq directoryPending
    /\ \A child \in directoryPending : DirectoryAncestors[child] \subseteq directoryPending
    /\ \A leaf \in DirectoryLeaves : directoryBindings[leaf] = DirectoryPreparedIdentity
DirectorySafety == directoryViolations = {}
DirectoryInvariant == DirectoryTypeOK /\ DirectoryEvidence /\ DirectorySafety
=============================================================================
