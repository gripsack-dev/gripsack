---- MODULE IncarnationExchange ----
\* Distinct immutable generation receipts may name different directory inodes
\* at one pathname. This is not a single-identity Forward transition.
\* A durable Exchange intent captures the immediate parent (Absent/File/Link
\* or the current directory inode). Rollback preserves the retained target
\* inode by staging it, then restores the captured parent and its leaf priors.
EXTENDS Naturals, FiniteSets
CONSTANTS LeafNames, PriorKind, Fault
ASSUME /\ IsFiniteSet(LeafNames) /\ PriorKind \in {"absent", "file", "link", "directory"}
\* Opaque qualified filesystem-kind/FSID/inode/birth-sec/birth-nsec tokens.
Current == [kind |-> "directory", identity |-> <<1, 101, 11, 1000, 1>>]
Target == [kind |-> "directory", identity |-> <<1, 101, 22, 2000, 1>>]
Foreign == [kind |-> "directory", identity |-> <<1, 202, 22, 2000, 1>>]
Absent == [kind |-> "absent"]
PriorFile == [kind |-> "file", content |-> "captured-file-bytes-mode"]
PriorLink == [kind |-> "link", target |-> "captured-exact-link-target"]
Original == CASE PriorKind = "directory" -> Current
             [] PriorKind = "file" -> PriorFile
             [] PriorKind = "link" -> PriorLink
             [] OTHER -> Absent
Values == {"absent", "prior", "deployed", "foreign"}
OriginalLeaves == [n \in LeafNames |-> IF PriorKind = "directory" THEN "prior" ELSE "absent"]
Disk == [parent: {Current, Target, Foreign, Absent, PriorFile, PriorLink},
         currentStage: BOOLEAN, targetStage: BOOLEAN, intent: BOOLEAN,
         currentLeaves: [LeafNames -> Values], targetLeaves: [LeafNames -> Values],
         currentEntries: SUBSET LeafNames, targetEntries: SUBSET LeafNames]
VARIABLES live, stable, mode, foreignParent, foreignLeaves, foreignStageChildren, violations
vars == <<live, stable, mode, foreignParent, foreignLeaves, foreignStageChildren, violations>>
Environment == <<foreignParent, foreignLeaves, foreignStageChildren>>
Init ==
    /\ live = [parent |-> Original, currentStage |-> FALSE, targetStage |-> TRUE,
                intent |-> FALSE, currentLeaves |-> OriginalLeaves,
                targetLeaves |-> [n \in LeafNames |-> "absent"],
                currentEntries |-> {}, targetEntries |-> {}]
    /\ stable = live /\ mode = "writing"
    /\ foreignParent = FALSE /\ foreignLeaves = {} /\ foreignStageChildren = {} /\ violations = {}
RecordExchange ==
    /\ mode = "writing" /\ ~live.intent /\ live.parent = Original
    /\ live' = [live EXCEPT !.intent = TRUE]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
SealIntent ==
    /\ live.intent # stable.intent
    /\ stable' = [stable EXCEPT !.intent = live.intent]
    /\ UNCHANGED <<live, mode, Environment, violations>>
RecordCurrent(n) ==
    /\ mode = "writing" /\ live.parent = Current /\ stable.intent
    /\ n \notin live.currentEntries
    /\ live' = [live EXCEPT !.currentEntries = @ \union {n}]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
SealEntries ==
    /\ <<live.currentEntries, live.targetEntries>> # <<stable.currentEntries, stable.targetEntries>>
    /\ stable' = [stable EXCEPT !.currentEntries = live.currentEntries,
                                !.targetEntries = live.targetEntries]
    /\ UNCHANGED <<live, mode, Environment, violations>>
RemoveCurrentLeaf(n) ==
    /\ mode = "writing" /\ live.parent = Current /\ stable.intent
    /\ n \in live.currentEntries \intersect stable.currentEntries
    /\ live.currentLeaves[n] = "prior"
    /\ live' = [live EXCEPT !.currentLeaves[n] = "absent"]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
SealLeaves(n) ==
    /\ <<live.currentLeaves[n], live.targetLeaves[n]>> # <<stable.currentLeaves[n], stable.targetLeaves[n]>>
    /\ stable' = [stable EXCEPT !.currentLeaves[n] = live.currentLeaves[n],
                                !.targetLeaves[n] = live.targetLeaves[n]]
    /\ UNCHANGED <<live, mode, Environment, violations>>
CurrentEmpty == \A n \in LeafNames :
    live.currentLeaves[n] = "absent" /\ stable.currentLeaves[n] = "absent"
TargetEmpty == \A n \in LeafNames :
    live.targetLeaves[n] = "absent" /\ stable.targetLeaves[n] = "absent"
StageCurrent ==
    /\ mode = "writing" /\ live.parent = Current /\ CurrentEmpty /\ stable.intent
    /\ "current" \notin foreignStageChildren
    /\ live' = [live EXCEPT !.parent = Absent, !.currentStage = TRUE]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
RemoveCapturedParent ==
    /\ mode = "writing" /\ PriorKind \in {"file", "link"}
    /\ live.parent = Original /\ stable.intent
    /\ live' = [live EXCEPT !.parent = Absent]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
InstallTarget ==
    /\ mode = "writing" /\ live.parent = Absent /\ live.targetStage /\ live.intent
    /\ (stable.intent \/ Fault = "missing-exchange")
    /\ ("target" \notin foreignStageChildren \/ Fault = "target-nonempty")
    /\ live' = [live EXCEPT !.parent = Target, !.targetStage = FALSE]
    /\ violations' = violations \union (IF stable.intent THEN {} ELSE {"intent"})
          \union (IF "target" \in foreignStageChildren THEN {"target-empty"} ELSE {})
    /\ UNCHANGED <<stable, mode, Environment>>
SealTree ==
    /\ <<live.parent, live.currentStage, live.targetStage>> #
       <<stable.parent, stable.currentStage, stable.targetStage>>
    /\ stable' = [stable EXCEPT !.parent = live.parent,
             !.currentStage = live.currentStage, !.targetStage = live.targetStage]
    /\ UNCHANGED <<live, mode, Environment, violations>>
RecordTarget(n) ==
    /\ mode = "writing" /\ live.parent = Target /\ stable.parent = Target
    /\ n \notin live.targetEntries /\ stable.intent
    /\ live' = [live EXCEPT !.targetEntries = @ \union {n}]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
WriteTargetLeaf(n) ==
    /\ mode = "writing" /\ live.parent = Target
    /\ n \in live.targetEntries \intersect stable.targetEntries
    /\ live.targetLeaves[n] = "absent"
    /\ live' = [live EXCEPT !.targetLeaves[n] = "deployed"]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
Commit ==
    /\ mode = "writing" /\ live.parent = Target /\ stable.parent = Target
    /\ \A n \in LeafNames : live.targetLeaves[n] = "deployed" /\ stable.targetLeaves[n] = "deployed"
    /\ mode' = "committed"
    /\ UNCHANGED <<live, stable, Environment, violations>>
StartRecovery ==
    /\ mode = "writing" /\ mode' = "recovering"
    /\ UNCHANGED <<live, stable, Environment, violations>>
Crash(power) ==
    /\ mode \in {"writing", "recovering"}
    /\ live' = IF power THEN stable ELSE live
    /\ mode' = "recovering"
    /\ UNCHANGED <<stable, Environment, violations>>
RemoveTargetLeaf(n) ==
    /\ mode = "recovering" /\ live.parent = Target
    /\ n \in live.targetEntries /\ live.targetLeaves[n] = "deployed"
    /\ live' = [live EXCEPT !.targetLeaves[n] = "absent"]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
StageTarget ==
    /\ mode = "recovering" /\ live.parent = Target
    /\ "target" \notin foreignStageChildren
    /\ (TargetEmpty \/ Fault = "stage-target-nonempty")
    /\ live' = [live EXCEPT !.parent = Absent,
             !.targetStage = IF Fault = "destroy-target" THEN FALSE ELSE TRUE]
    /\ violations' = violations \union (IF TargetEmpty THEN {} ELSE {"nonempty"})
    /\ UNCHANGED <<stable, mode, Environment>>
RestoreCapturedParent ==
    /\ mode = "recovering" /\ live.parent = Absent
    /\ live.targetStage /\ stable.targetStage
    /\ (PriorKind # "directory" \/ live.currentStage)
    /\ live' = [live EXCEPT
             !.parent = IF Fault = "wrong-prior" THEN PriorLink ELSE Original,
             !.currentStage = FALSE]
    /\ violations' = violations \union
         (IF Fault = "wrong-prior" /\ Original # PriorLink THEN {"prior"} ELSE {})
    /\ UNCHANGED <<stable, mode, Environment>>
RestoreCurrentLeaf(n) ==
    /\ mode = "recovering" /\ n \in live.currentEntries
    /\ ((live.parent = Current /\ stable.parent = Current) \/ Fault = "wrong-incarnation")
    /\ live.currentLeaves[n] = "absent"
    /\ live' = IF live.parent = Target
                THEN [live EXCEPT !.targetLeaves[n] = "prior"]
                ELSE [live EXCEPT !.currentLeaves[n] = "prior"]
    /\ violations' = violations \union
         (IF live.parent = Current /\ stable.parent = Current THEN {} ELSE {"ancestry"})
    /\ UNCHANGED <<stable, mode, Environment>>
ClearTargetEntry(n) ==
    /\ mode = "recovering" /\ n \in live.targetEntries
    /\ live.targetLeaves[n] = "absent" /\ stable.targetLeaves[n] = "absent"
    /\ live' = [live EXCEPT !.targetEntries = @ \ {n}]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
ClearCurrentEntry(n) ==
    /\ mode = "recovering" /\ n \in live.currentEntries
    /\ live.parent = Current /\ stable.parent = Current
    /\ live.currentLeaves[n] = "prior" /\ stable.currentLeaves[n] = "prior"
    /\ live' = [live EXCEPT !.currentEntries = @ \ {n}]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
Restored ==
    /\ live.parent = Original /\ stable.parent = Original
    /\ live.currentLeaves = OriginalLeaves /\ stable.currentLeaves = OriginalLeaves
    /\ TargetEmpty /\ live.targetStage /\ stable.targetStage
    /\ ~live.currentStage /\ ~stable.currentStage
EntriesEmpty == live.currentEntries = {} /\ stable.currentEntries = {}
                /\ live.targetEntries = {} /\ stable.targetEntries = {}
ClearExchange ==
    /\ mode = "recovering" /\ live.intent /\ Restored /\ EntriesEmpty
    /\ live' = [live EXCEPT !.intent = FALSE]
    /\ UNCHANGED <<stable, mode, Environment, violations>>
Finish ==
    /\ mode = "recovering" /\ ~live.intent /\ ~stable.intent /\ Restored /\ EntriesEmpty
    /\ mode' = "done"
    /\ UNCHANGED <<live, stable, Environment, violations>>
ReplaceParent ==
    /\ mode = "recovering" /\ live.intent /\ stable.intent /\ ~foreignParent
    /\ live' = [live EXCEPT !.parent = Foreign]
    /\ stable' = [stable EXCEPT !.parent = Foreign]
    /\ foreignParent' = TRUE /\ UNCHANGED <<mode, foreignLeaves, foreignStageChildren, violations>>
EditTargetLeaf(n) ==
    /\ mode = "recovering" /\ live.parent = Target /\ stable.parent = Target
    /\ n \notin foreignLeaves
    /\ live' = [live EXCEPT !.targetLeaves[n] = "foreign"]
    /\ stable' = [stable EXCEPT !.targetLeaves[n] = "foreign"]
    /\ foreignLeaves' = foreignLeaves \union {n}
    /\ UNCHANGED <<mode, foreignParent, foreignStageChildren, violations>>
AddStageChild(which) ==
    /\ mode \in {"writing", "recovering"} /\ which \notin foreignStageChildren
    /\ IF which = "current" THEN live.currentStage /\ stable.currentStage
       ELSE live.targetStage /\ stable.targetStage
    /\ foreignStageChildren' = foreignStageChildren \union {which}
    /\ UNCHANGED <<live, stable, mode, foreignParent, foreignLeaves, violations>>
Next == RecordExchange \/ SealIntent \/ SealEntries \/ StageCurrent \/ RemoveCapturedParent
        \/ InstallTarget \/ SealTree \/ Commit \/ StartRecovery \/ Crash(TRUE) \/ Crash(FALSE)
        \/ StageTarget \/ RestoreCapturedParent \/ ClearExchange \/ Finish \/ ReplaceParent
        \/ (\E which \in {"current", "target"} : AddStageChild(which))
        \/ (\E n \in LeafNames : RecordCurrent(n) \/ RemoveCurrentLeaf(n) \/ SealLeaves(n)
             \/ RecordTarget(n) \/ WriteTargetLeaf(n) \/ RemoveTargetLeaf(n)
             \/ RestoreCurrentLeaf(n) \/ ClearTargetEntry(n) \/ ClearCurrentEntry(n) \/ EditTargetLeaf(n))
        \/ UNCHANGED vars
Spec == Init /\ [][Next]_vars
TypeOK == live \in Disk /\ stable \in Disk
          /\ mode \in {"writing", "recovering", "committed", "done"}
          /\ foreignParent \in BOOLEAN /\ foreignLeaves \subseteq LeafNames
          /\ foreignStageChildren \subseteq {"current", "target"}
          /\ violations \subseteq {"intent", "nonempty", "prior", "ancestry", "target-empty"}
ExchangeIntentBeforeInstall == "intent" \notin violations
OnlyEmptyTargetStaged == "nonempty" \notin violations
ExactCapturedPrior == "prior" \notin violations
LeafBindsIncarnation == "ancestry" \notin violations
OnlyEmptyTargetInstalled == "target-empty" \notin violations
OriginalStageChildrenPreserved == "current" \in foreignStageChildren /\ ~foreignParent =>
    (live.parent = Current \/ live.currentStage) /\ (stable.parent = Current \/ stable.currentStage)
RetainedTargetNeverDestroyed == ~foreignParent =>
    (live.parent = Target \/ live.targetStage) /\ (stable.parent = Target \/ stable.targetStage)
OriginalDirectoryNeverDestroyed == PriorKind = "directory" /\ ~foreignParent =>
    (live.parent = Current \/ live.currentStage) /\ (stable.parent = Current \/ stable.currentStage)
ForeignPreserved ==
    /\ (foreignParent => live.parent = Foreign /\ stable.parent = Foreign)
    /\ \A n \in foreignLeaves : live.targetLeaves[n] = "foreign" /\ stable.targetLeaves[n] = "foreign"
ConflictRetainsEvidence == (foreignParent \/ foreignLeaves # {}) => live.intent /\ stable.intent
CompletedExactly == mode = "done" => Restored /\ EntriesEmpty /\ ~live.intent /\ ~stable.intent
Safety == ExchangeIntentBeforeInstall /\ OnlyEmptyTargetStaged /\ ExactCapturedPrior
          /\ LeafBindsIncarnation /\ RetainedTargetNeverDestroyed /\ OriginalDirectoryNeverDestroyed
          /\ ForeignPreserved /\ ConflictRetainsEvidence /\ CompletedExactly
          /\ OnlyEmptyTargetInstalled /\ OriginalStageChildrenPreserved
NeverTargetInstalled == live.parent # Target
NeverExchangeCommitted == mode # "committed"
NeverBothIncarnations == ~(live.parent = Target /\ live.currentStage)
NeverTargetRestaged == ~(mode = "recovering" /\ live.parent = Absent
                        /\ live.targetStage /\ stable.parent = Target)
NeverOriginalStageChildRestored == ~("current" \in foreignStageChildren
                                    /\ live.parent = Current /\ mode = "recovering")
=============================================================================
