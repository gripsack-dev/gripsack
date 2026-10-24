---- MODULE ReverseDirectoryHierarchy ----
\* Journal-v3 Reverse is not Forward with an inverted final value. Forward
\* recovery removes children before rmdir; Reverse recovery must reinstall
\* the staged SAME inode before restoring child priors. Bound leaf entries
\* retain that ancestor identity across either direction.
EXTENDS Naturals, FiniteSets
CONSTANTS LeafNames, OriginalTarget, OwnedIdentity, ReplacementIdentity, Fault
ASSUME /\ IsFiniteSet(LeafNames) /\ OwnedIdentity # ReplacementIdentity
\* Opaque qualified filesystem-kind/FSID/inode/birth-sec/birth-nsec tokens.
OwnedInode == <<1, 101, 11, 1000, 1>>
OtherFilesystem == <<1, 202, 11, 1000, 1>>
Original == [kind |-> "link", identity |-> OriginalTarget]
Owned == [kind |-> "directory", identity |-> OwnedIdentity]
Foreign == [kind |-> "directory", identity |-> ReplacementIdentity]
Missing == [kind |-> "absent"]
OlderFile == [kind |-> "file", content |-> "older-file"]
Values == {"prior", "absent", "foreign"}
Disk == [parent: {Original, Owned, Foreign, Missing, OlderFile},
         staged: {Owned, Foreign, Missing}, leaves: [LeafNames -> Values],
         entries: SUBSET LeafNames, intent: BOOLEAN, committed: BOOLEAN,
         activation: BOOLEAN, rootEntry: BOOLEAN,
         rootAfter: {Original, Missing, OlderFile}]
VARIABLES live, stable, mode, foreignParent, foreignStage, foreignLeaves,
          foreignOriginalChild, rootReady, violations
vars == <<live, stable, mode, foreignParent, foreignStage, foreignLeaves,
          foreignOriginalChild, rootReady, violations>>
Environment == <<foreignParent, foreignStage, foreignLeaves, foreignOriginalChild, rootReady>>
Init ==
    /\ live = [parent |-> Owned, staged |-> Missing,
                leaves |-> [n \in LeafNames |-> "prior"], entries |-> {},
                intent |-> FALSE, committed |-> FALSE, activation |-> FALSE,
                rootEntry |-> FALSE, rootAfter |-> Original]
    /\ stable = live /\ mode = "writing"
    /\ foreignParent = FALSE /\ foreignStage = FALSE /\ foreignLeaves = {}
    /\ violations = {} /\ rootReady = FALSE /\ foreignOriginalChild = FALSE
RecordReverse ==
    /\ mode = "writing" /\ ~live.intent /\ live.parent = Owned
    /\ live' = [live EXCEPT !.intent = TRUE]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
SealIntent ==
    /\ live.intent # stable.intent
    /\ stable' = [stable EXCEPT !.intent = live.intent]
    /\ UNCHANGED <<live, mode, Environment, violations>>
RecordBound(n) ==
    /\ mode = "writing" /\ stable.intent /\ live.parent = Owned
    /\ n \notin live.entries /\ live.leaves[n] = "prior"
    /\ live' = [live EXCEPT !.entries = @ \union {n}]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
SealEntries ==
    /\ live.entries # stable.entries
    /\ stable' = [stable EXCEPT !.entries = live.entries]
    /\ UNCHANGED <<live, mode, Environment, violations>>
RemoveLeaf(n) ==
    /\ mode = "writing" /\ live.parent = Owned
    /\ n \in live.entries \intersect stable.entries /\ stable.intent
    /\ live.leaves[n] = "prior"
    /\ live' = [live EXCEPT !.leaves[n] = "absent"]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
SealLeaf(n) ==
    /\ live.leaves[n] # stable.leaves[n]
    /\ stable' = [stable EXCEPT !.leaves[n] = live.leaves[n]]
    /\ UNCHANGED <<live, mode, Environment, violations>>
Empty == \A n \in LeafNames : live.leaves[n] = "absent" /\ stable.leaves[n] = "absent"
StageDirectory ==
    /\ mode = "writing" /\ live.intent
    /\ (stable.intent \/ Fault = "missing-reverse-intent")
    /\ live.parent = Owned /\ live.staged = Missing
    /\ (Empty \/ Fault = "stage-nonempty")
    /\ live' = [live EXCEPT !.parent = Missing, !.staged = Owned]
    /\ violations' = violations
         \union (IF Empty THEN {} ELSE {"nonempty"})
         \union (IF stable.intent THEN {} ELSE {"intent"})
    /\ UNCHANGED <<stable, mode, Environment>>
\* The rename is one namespace identity transition. SealTree acknowledges
\* syncing BOTH source and destination parents; it is not a single-fsync claim.
SealTree ==
    /\ <<live.parent, live.staged>> # <<stable.parent, stable.staged>>
    /\ stable' = [stable EXCEPT !.parent = live.parent, !.staged = live.staged]
    /\ UNCHANGED <<live, mode, Environment, violations>>
RestoreLink ==
    /\ mode = "writing" /\ live.parent = Missing /\ live.staged = Owned
    /\ ~live.rootEntry
    /\ live' = [live EXCEPT !.parent = Original]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
\* An older rollback target may need an ordinary flat parent write AFTER the
\* reverse transition restored its predecessor link. Its original prior is
\* still that exact link, not the staged directory or the older target.
RecordEarlierRoot ==
    /\ mode = "writing" /\ live.parent = Original /\ stable.parent = Original
    /\ live.staged = Owned /\ ~live.rootEntry
    /\ \E after \in {Missing, OlderFile} :
         live' = [live EXCEPT !.rootEntry = TRUE, !.rootAfter = after]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
SealRootEntry ==
    /\ <<live.rootEntry, live.rootAfter>> # <<stable.rootEntry, stable.rootAfter>>
    /\ stable' = [stable EXCEPT !.rootEntry = live.rootEntry, !.rootAfter = live.rootAfter]
    /\ UNCHANGED <<live, mode, Environment, violations>>
WriteEarlierRoot ==
    /\ mode = "writing" /\ live.rootEntry /\ stable.rootEntry
    /\ live.rootAfter = stable.rootAfter /\ live.parent = Original
    /\ live' = [live EXCEPT !.parent = live.rootAfter]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
Commit ==
    /\ mode = "writing" /\ Empty
    /\ live.rootAfter = stable.rootAfter
    /\ live.parent = live.rootAfter /\ stable.parent = stable.rootAfter
    /\ live.staged = Owned /\ stable.staged = Owned
    /\ live.intent /\ stable.intent
    /\ live' = [live EXCEPT !.committed = TRUE]
    /\ stable' = [stable EXCEPT !.committed = TRUE]
    /\ mode' = "recovering" /\ UNCHANGED <<Environment, violations>>
StartRecovery ==
    /\ mode = "writing" /\ mode' = "recovering"
    /\ UNCHANGED <<live, stable, Environment, violations>>
Crash(power) ==
    /\ mode \in {"writing", "recovering"}
    /\ live' = IF power THEN stable ELSE live
    /\ mode' = "recovering" /\ rootReady' = FALSE
    /\ UNCHANGED <<stable, foreignParent, foreignStage, foreignLeaves, foreignOriginalChild, violations>>
\* Never replay ordinary root undo over an already-restored exact directory.
NormalizeRoot ==
    /\ mode = "recovering" /\ ~stable.committed /\ live.rootEntry
    /\ ((~rootReady /\ live.parent \in {live.rootAfter, Missing}
         /\ live.staged = Owned)
        \/ (Fault = "root-replay" /\ live.parent = Owned))
    /\ live' = [live EXCEPT !.parent = Original]
    /\ violations' = violations \union
         (IF live.parent = Owned THEN {"root-replay"} ELSE {})
    /\ UNCHANGED <<stable, mode, Environment>>
UnlinkForRestore ==
    /\ mode = "recovering" /\ ~stable.committed
    /\ live.parent = Original /\ live.staged = Owned
    /\ (~live.rootEntry \/ stable.parent = Original)
    /\ live' = [live EXCEPT !.parent = Missing] /\ rootReady' = TRUE
    /\ UNCHANGED <<stable, mode, foreignParent, foreignStage, foreignLeaves, foreignOriginalChild, violations>>
ReinstallParent ==
    /\ mode = "recovering" /\ ~stable.committed
    /\ live.parent = Missing
    /\ (~live.rootEntry \/ rootReady \/ Fault = "parent-before-root")
    /\ (live.staged = Owned
        \/ (Fault = "foreign-stage" /\ live.staged = Foreign))
    /\ live' = [live EXCEPT
         !.parent = IF live.staged = Owned THEN Owned ELSE Foreign,
         !.staged = Missing]
    /\ violations' = violations \union
         (IF ~live.rootEntry \/ rootReady THEN {} ELSE {"root-order"})
    /\ UNCHANGED <<stable, mode, Environment>>
RestoreLeaf(n) ==
    /\ mode = "recovering" /\ ~stable.committed /\ n \in live.entries
    /\ ((live.parent = Owned /\ stable.parent = Owned) \/ Fault = "child-first")
    /\ live.leaves[n] = "absent"
    /\ live' = [live EXCEPT !.leaves[n] = "prior"]
    /\ violations' = violations \union
         (IF live.parent = Owned /\ stable.parent = Owned THEN {} ELSE {"ancestry"})
    /\ UNCHANGED <<stable, mode, Environment>>
ClearEntry(n) ==
    /\ mode = "recovering" /\ n \in live.entries
    /\ IF stable.committed THEN Empty
       ELSE live.parent = Owned /\ stable.parent = Owned
            /\ live.leaves[n] = "prior" /\ stable.leaves[n] = "prior"
    /\ live' = [live EXCEPT !.entries = @ \ {n}]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
PriorRestored ==
    /\ live.parent = Owned /\ stable.parent = Owned
    /\ live.staged = Missing /\ stable.staged = Missing
    /\ \A n \in LeafNames : live.leaves[n] = "prior" /\ stable.leaves[n] = "prior"
ReverseCommitted ==
    /\ live.rootAfter = stable.rootAfter
    /\ live.parent = live.rootAfter /\ stable.parent = stable.rootAfter /\ Empty
    /\ live.staged = Owned /\ stable.staged = Owned
ClearRootEntry ==
    /\ mode = "recovering" /\ live.rootEntry
    /\ IF stable.committed THEN ReverseCommitted ELSE PriorRestored
    /\ live' = [live EXCEPT !.rootEntry = FALSE]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
ClearReverse ==
    /\ mode = "recovering" /\ live.intent
    /\ live.entries = {} /\ stable.entries = {}
    /\ ~live.rootEntry /\ ~stable.rootEntry
    /\ IF stable.committed THEN ReverseCommitted ELSE PriorRestored
    /\ live' = [live EXCEPT !.intent = FALSE]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
Finish ==
    /\ mode = "recovering" /\ ~live.intent /\ ~stable.intent
    /\ live.entries = {} /\ stable.entries = {}
    /\ ~live.rootEntry /\ ~stable.rootEntry
    /\ IF stable.committed THEN ReverseCommitted ELSE PriorRestored
    /\ mode' = "done"
    /\ UNCHANGED <<live, stable, Environment, violations>>
ReplaceParent ==
    /\ mode = "recovering" /\ ~stable.committed /\ live.intent /\ stable.intent
    /\ live.parent \in {Owned, Missing, Original, OlderFile} /\ ~foreignParent
    /\ live' = [live EXCEPT !.parent = Foreign]
    /\ stable' = [stable EXCEPT !.parent = Foreign]
    /\ foreignParent' = TRUE
    /\ UNCHANGED <<mode, foreignStage, foreignLeaves, foreignOriginalChild, rootReady, violations>>
ReplaceStage ==
    /\ mode = "recovering" /\ live.intent /\ stable.intent
    /\ live.staged = Owned /\ ~foreignStage
    /\ live' = [live EXCEPT !.staged = Foreign]
    /\ stable' = [stable EXCEPT !.staged = Foreign]
    /\ foreignStage' = TRUE
    /\ UNCHANGED <<mode, foreignParent, foreignLeaves, foreignOriginalChild, rootReady, violations>>
EditLeaf(n) ==
    /\ mode = "recovering" /\ live.parent = Owned /\ stable.parent = Owned
    /\ live.intent /\ stable.intent /\ n \notin foreignLeaves
    /\ live' = [live EXCEPT !.leaves[n] = "foreign"]
    /\ stable' = [stable EXCEPT !.leaves[n] = "foreign"]
    /\ foreignLeaves' = foreignLeaves \union {n}
    /\ UNCHANGED <<mode, foreignParent, foreignStage, foreignOriginalChild, rootReady, violations>>
\* Foreign contents do not replace the original container's identity.
\* Compensation may move that exact container home without deleting a child.
AddOriginalStageChild ==
    /\ mode = "recovering" /\ ~foreignOriginalChild
    /\ live.staged = Owned /\ stable.staged = Owned
    /\ foreignOriginalChild' = TRUE
    /\ UNCHANGED <<live, stable, mode, foreignParent, foreignStage, foreignLeaves, rootReady, violations>>
\* The retained stage is usable only under the next durable activation intent.
RecordReactivation ==
    /\ mode = "done" /\ stable.committed /\ ReverseCommitted
    /\ live.parent = Original
    /\ (~foreignOriginalChild \/ Fault = "target-nonempty")
    /\ live' = [live EXCEPT !.activation = TRUE] /\ mode' = "activating"
    /\ UNCHANGED <<stable, Environment, violations>>
SealActivation ==
    /\ mode = "activating" /\ live.activation /\ ~stable.activation
    /\ stable' = [stable EXCEPT !.activation = TRUE]
    /\ UNCHANGED <<live, mode, Environment, violations>>
Reactivate ==
    /\ mode = "activating" /\ stable.activation
    /\ live.parent = Original /\ live.staged = Owned
    /\ (~foreignOriginalChild \/ Fault = "target-nonempty")
    /\ live' = [live EXCEPT !.parent = Owned, !.staged = Missing]
    /\ mode' = "reactivated"
    /\ violations' = violations \union (IF foreignOriginalChild THEN {"target-empty"} ELSE {})
    /\ UNCHANGED <<stable, Environment>>
DropCommittedStage ==
    /\ Fault = "drop-stage" /\ stable.committed /\ ~stable.activation
    /\ live' = [live EXCEPT !.staged = Missing]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
Next == RecordReverse \/ SealIntent \/ StageDirectory \/ SealTree \/ RestoreLink
        \/ Commit \/ StartRecovery \/ Crash(TRUE) \/ Crash(FALSE)
        \/ UnlinkForRestore \/ ReinstallParent \/ ClearReverse \/ Finish
        \/ ReplaceParent \/ ReplaceStage \/ RecordReactivation \/ SealActivation \/ Reactivate
        \/ DropCommittedStage \/ SealEntries \/ AddOriginalStageChild
        \/ RecordEarlierRoot \/ SealRootEntry \/ WriteEarlierRoot
        \/ NormalizeRoot \/ ClearRootEntry
        \/ (\E n \in LeafNames : RecordBound(n) \/ RemoveLeaf(n) \/ SealLeaf(n)
                                 \/ RestoreLeaf(n) \/ ClearEntry(n) \/ EditLeaf(n))
        \/ UNCHANGED vars
Spec == Init /\ [][Next]_vars
TypeOK ==
    /\ live \in Disk /\ stable \in Disk
    /\ mode \in {"writing", "recovering", "done", "activating", "reactivated"}
    /\ foreignParent \in BOOLEAN /\ foreignStage \in BOOLEAN /\ foreignLeaves \subseteq LeafNames
    /\ rootReady \in BOOLEAN /\ foreignOriginalChild \in BOOLEAN
    /\ violations \subseteq {"nonempty", "ancestry", "intent", "root-replay", "root-order", "target-empty"}
EmptyBeforeStaging == "nonempty" \notin violations
ParentBeforeChildRestore == "ancestry" \notin violations
ReverseMutationHasIntent == "intent" \notin violations
RootUndoNeverReplacesRestoredDirectory == "root-replay" \notin violations
RootNormalizedBeforeParentRestore == "root-order" \notin violations
TargetReactivationEmpty == "target-empty" \notin violations
OriginalStageChildPreserved == foreignOriginalChild /\ ~foreignParent /\ ~foreignStage =>
    (live.parent = Owned \/ live.staged = Owned)
    /\ (stable.parent = Owned \/ stable.staged = Owned)
CommittedStageRetained == stable.committed /\ ~stable.activation /\ ~foreignStage =>
    live.staged = Owned /\ stable.staged = Owned
ForeignPreserved ==
    /\ (foreignParent => live.parent = Foreign /\ stable.parent = Foreign)
    /\ (foreignStage => live.staged = Foreign /\ stable.staged = Foreign)
    /\ \A n \in foreignLeaves : live.leaves[n] = "foreign" /\ stable.leaves[n] = "foreign"
ConflictRetainsEvidence == (foreignParent \/ foreignStage \/ foreignLeaves # {}) =>
    live.intent /\ stable.intent
CompleteExactly == mode = "done" =>
    IF stable.committed THEN ReverseCommitted ELSE PriorRestored
SameInodeReactivated == mode = "reactivated" =>
    live.parent = Owned /\ live.staged = Missing /\ stable.activation
Safety == EmptyBeforeStaging /\ ParentBeforeChildRestore /\ CommittedStageRetained
          /\ ForeignPreserved /\ ConflictRetainsEvidence /\ CompleteExactly
          /\ SameInodeReactivated /\ ReverseMutationHasIntent
          /\ RootUndoNeverReplacesRestoredDirectory /\ RootNormalizedBeforeParentRestore
          /\ TargetReactivationEmpty /\ OriginalStageChildPreserved
NeverStaged == live.staged # Owned
NeverReactivated == mode # "reactivated"
NeverOlderFileTarget == live.parent # OlderFile
NeverOlderAbsentTarget == ~(live.rootEntry /\ live.parent = Missing /\ mode = "writing")
NeverOriginalStageChildRestored == ~(foreignOriginalChild /\ live.parent = Owned /\ mode = "recovering")
=============================================================================
