---- MODULE DirectoryEvolution ----
\* Ordinary post-cutover repoTree changes retain one logical root while owned
\* subdirectories grow, retire and change incarnation. Existing real ancestors
\* are borrowed capabilities, never directory-removal authority. An unrelated
\* foreign root child must not prevent a safe change below another branch.
\* Each stage is an exclusive sibling namespace outside the root, so staging
\* a descendant never hides it inside an ancestor that is about to move.
EXTENDS Naturals, FiniteSets
CONSTANTS Change, ForeignSibling, BorrowedPresent, Fault
ASSUME /\ Change \in {"grow", "remove", "retarget", "reverse"}
       /\ ForeignSibling \in BOOLEAN /\ BorrowedPresent \in BOOLEAN
Slots == {"root", "old", "added", "nested", "borrowed"}
\* Each opaque token denotes full filesystem kind + fstatvfs FSID + inode +
\* required statx birth seconds/nanoseconds, not st_dev or a mount identifier.
Ids == {"root-id", "old-id", "replacement-id", "added-id", "nested-id", "borrowed-id"}
Owned == Ids \ {"borrowed-id"}
Absent == "absent"
Initial == [s \in Slots |-> CASE s = "root" -> "root-id"
             [] s = "old" -> "old-id"
             [] s = "borrowed" /\ BorrowedPresent -> "borrowed-id"
             [] OTHER -> Absent]
Requested == [s \in Slots |-> CASE
      Change = "reverse" -> Absent
   [] s = "old" /\ Change = "remove" -> Absent
   [] s = "old" /\ Change = "retarget" -> "replacement-id"
   [] s = "added" /\ Change = "grow" -> "added-id"
   [] s = "nested" /\ Change = "grow" -> "nested-id"
   [] OTHER -> Initial[s]]
Range(shape) == {shape[s] : s \in Slots} \ {Absent}
Ancestors(id) == CASE id = "root-id" -> {}
    [] id = "nested-id" -> {"root-id", "added-id"}
    [] OTHER -> {"root-id"}
SlotAncestors(s) == CASE s = "root" -> {}
    [] s = "nested" -> {"root", "added"}
    [] OTHER -> {"root"}
Payload(id) == IF id \in Range(Initial) THEN "prior" ELSE "new"
InitialContents == [id \in Ids |-> IF id \in Range(Initial) THEN Payload(id) ELSE Absent]
Contents == [Ids -> {Absent, "prior", "new", "foreign"}]
Shapes == [Slots -> Ids \union {Absent}]
VARIABLES shape, stableShape, stages, stableStages, contents, stableContents,
          intent, stableIntent, prepared, protected, entries, before, baseline,
          target, mode, round, recoveryRestarts, foreignNodes, foreignAt, foreignLeaves,
          foreignStageChildren, violations
vars == <<shape, stableShape, stages, stableStages, contents, stableContents,
          intent, stableIntent, prepared, protected, entries, before, baseline,
          target, mode, round, recoveryRestarts, foreignNodes, foreignAt, foreignLeaves,
          foreignStageChildren, violations>>
Tree == <<shape, stableShape, stages, stableStages>>
Leaves == <<contents, stableContents>>
Journal == <<intent, stableIntent, entries, before, baseline>>
Control == <<target, mode, round, recoveryRestarts>>
Environment == <<foreignNodes, foreignAt, foreignLeaves, foreignStageChildren>>
Init ==
    /\ shape = Initial /\ stableShape = Initial /\ stages = {} /\ stableStages = {}
    /\ contents = InitialContents /\ stableContents = contents
    /\ intent = FALSE /\ stableIntent = FALSE /\ entries = {}
    /\ before = Initial /\ baseline = contents /\ target = Requested
    /\ prepared = Range(Initial) \intersect Owned /\ protected = prepared
    /\ mode = "writing" /\ round = 0 /\ recoveryRestarts = 0
    /\ foreignNodes = IF ForeignSibling THEN {"root"} ELSE {}
    /\ foreignAt = Initial /\ foreignLeaves = {} /\ foreignStageChildren = {} /\ violations = {}
Goal == IF mode = "recovering" THEN before ELSE target
Bound(id) == id \in Range(shape) /\ id \in Range(stableShape)
             /\ Ancestors(id) \subseteq Range(shape)
             /\ Ancestors(id) \subseteq Range(stableShape)
Prepare(id) ==
    /\ mode = "writing" /\ id \in Range(target) \intersect Owned
    /\ id \notin prepared /\ id \notin protected
    /\ prepared' = prepared \union {id}
    /\ stages' = stages \union {id} /\ stableStages' = stableStages \union {id}
    /\ UNCHANGED <<shape, stableShape, Leaves, Journal, protected, Control, Environment, violations>>
RecordEvolve ==
    /\ mode = "writing" /\ ~intent
    /\ Range(target) \intersect Owned \subseteq prepared
    /\ intent' = TRUE
    /\ UNCHANGED <<Tree, Leaves, stableIntent, entries, before, baseline,
                    prepared, protected, Control, Environment, violations>>
SealIntent ==
    /\ intent # stableIntent /\ stableIntent' = intent
    /\ UNCHANGED <<Tree, Leaves, intent, entries, before, baseline,
                    prepared, protected, Control, Environment, violations>>
RecordLeaf(id) ==
    /\ mode = "writing" /\ stableIntent /\ Bound(id) /\ id \notin entries
    /\ entries' = entries \union {id}
    /\ UNCHANGED <<Tree, Leaves, intent, stableIntent, before, baseline,
                    prepared, protected, Control, Environment, violations>>
ClearChangedLeaf(id) ==
    /\ mode \in {"writing", "recovering"} /\ id \in entries /\ Bound(id)
    /\ id \notin Range(Goal) /\ contents[id] # Absent
    /\ (contents[id] \in {baseline[id], Payload(id)} \/ Fault = "leaf-identity")
    /\ contents' = [contents EXCEPT ![id] = Absent]
    /\ UNCHANGED <<Tree, stableContents, Journal, prepared, protected, Control, Environment, violations>>
WriteDesiredLeaf(id) ==
    /\ mode = "writing" /\ id \in entries /\ Bound(id)
    /\ id \in Range(target) /\ contents[id] # Payload(id)
    /\ contents[id] \in {baseline[id], Absent}
    /\ contents' = [contents EXCEPT ![id] = Payload(id)]
    /\ UNCHANGED <<Tree, stableContents, Journal, prepared, protected, Control, Environment, violations>>
SealLeaf(id) ==
    /\ contents[id] # stableContents[id]
    /\ stableContents' = [stableContents EXCEPT ![id] = contents[id]]
    /\ UNCHANGED <<Tree, contents, Journal, prepared, protected, Control, Environment, violations>>
ChildrenAbsent(s) == \A child \in Slots : s \in SlotAncestors(child) =>
    shape[child] = Absent /\ stableShape[child] = Absent
Retire(s) ==
    /\ mode \in {"writing", "recovering"}
    /\ shape[s] # Absent
    /\ (shape[s] # Goal[s] \/ (Fault = "borrowed-delete" /\ s = "borrowed"))
    /\ (shape[s] \in Owned \/ Fault = "borrowed-delete")
    /\ intent /\ (stableIntent \/ Fault = "missing-barrier")
    /\ contents[shape[s]] = Absent /\ stableContents[shape[s]] = Absent
    /\ shape[s] \notin foreignStageChildren
    /\ (ChildrenAbsent(s) \/ Fault = "parent-first")
    /\ (s \notin foreignNodes \/ Fault = "foreign-child")
    /\ shape' = [shape EXCEPT ![s] = Absent]
    /\ stages' = stages \union {shape[s]}
    /\ violations' = violations
          \union (IF stableIntent THEN {} ELSE {"intent"})
          \union (IF ChildrenAbsent(s) THEN {} ELSE {"order"})
    /\ UNCHANGED <<stableShape, stableStages, Leaves, Journal,
                    prepared, protected, Control, Environment>>
Install(s) ==
    /\ mode \in {"writing", "recovering"} /\ shape[s] = Absent /\ Goal[s] # Absent
    /\ Goal[s] \in stages /\ Goal[s] \in prepared
    /\ intent /\ (stableIntent \/ Fault = "missing-barrier")
    /\ (mode = "recovering" \/ Goal[s] \notin foreignStageChildren \/ Fault = "target-nonempty")
    /\ ((Ancestors(Goal[s]) \subseteq Range(shape)
         /\ Ancestors(Goal[s]) \subseteq Range(stableShape)) \/ Fault = "ancestor")
    /\ shape' = [shape EXCEPT ![s] = Goal[s]] /\ stages' = stages \ {Goal[s]}
    /\ violations' = violations \union (IF stableIntent THEN {} ELSE {"intent"})
          \union (IF mode # "recovering" /\ Goal[s] \in foreignStageChildren
                  THEN {"target-empty"} ELSE {})
    /\ UNCHANGED <<stableShape, stableStages, Leaves, Journal,
                    prepared, protected, Control, Environment>>
\* A namespace transition acknowledges all affected source/staging parents.
SealTree ==
    /\ <<shape, stages>> # <<stableShape, stableStages>>
    /\ stableShape' = shape /\ stableStages' = stages
    /\ UNCHANGED <<shape, stages, Leaves, Journal, prepared, protected, Control, Environment, violations>>
Publish ==
    /\ mode = "writing" /\ Range(target) \intersect Owned \subseteq prepared
    /\ ~(Range(target) \intersect Owned \subseteq protected)
    /\ protected' = protected \union (Range(target) \intersect Owned)
    /\ UNCHANGED <<Tree, Leaves, Journal, prepared, Control, Environment, violations>>
TargetReady ==
    /\ shape = target /\ stableShape = target
    /\ \A id \in Range(target) : contents[id] = Payload(id) /\ stableContents[id] = Payload(id)
Commit ==
    /\ mode = "writing" /\ stableIntent /\ TargetReady
    /\ Range(target) \intersect Owned \subseteq protected
    /\ mode' = "committed"
    /\ UNCHANGED <<Tree, Leaves, Journal, prepared, protected, target, round, recoveryRestarts, Environment, violations>>
\* A retained older generation supplies the previous shape/payload recipe,
\* not the current snapshot. This is a real second evolution epoch.
BeginHistoryRollback ==
    /\ mode = "committed" /\ round = 0
    /\ before' = shape /\ baseline' = contents /\ target' = Initial
    /\ intent' = FALSE /\ stableIntent' = FALSE /\ entries' = {}
    /\ mode' = "writing" /\ round' = 1
    /\ UNCHANGED <<Tree, Leaves, prepared, protected, recoveryRestarts, Environment, violations>>
StartRecovery ==
    /\ mode = "writing" /\ mode' = "recovering"
    /\ UNCHANGED <<Tree, Leaves, Journal, prepared, protected, target, round, recoveryRestarts, Environment, violations>>
Crash(power) ==
    /\ mode \in {"writing", "recovering"}
    /\ shape' = IF power THEN stableShape ELSE shape
    /\ stages' = IF power THEN stableStages ELSE stages
    /\ contents' = IF power THEN stableContents ELSE contents
    /\ intent' = IF power THEN stableIntent ELSE intent
    /\ mode' = "recovering"
    \* Saturating witness history does not disable or bound any crash action.
    /\ recoveryRestarts' = IF mode = "recovering" /\ recoveryRestarts < 2
          /\ (shape # stableShape \/ contents # stableContents)
          THEN recoveryRestarts + 1 ELSE recoveryRestarts
    /\ UNCHANGED <<stableShape, stableStages, stableContents, stableIntent,
                    entries, before, baseline, prepared, protected, target, round, Environment, violations>>
RestoreLeaf(id) ==
    /\ mode = "recovering" /\ id \in entries /\ id \in Range(before)
    /\ ((shape = before /\ stableShape = before /\ Bound(id)) \/ Fault = "leaf-ancestor")
    /\ contents[id] # baseline[id]
    /\ contents[id] \in {baseline[id], Payload(id), Absent}
    /\ contents' = [contents EXCEPT ![id] = baseline[id]]
    /\ violations' = violations \union
          (IF shape = before /\ stableShape = before /\ Bound(id) THEN {} ELSE {"leaf"})
    /\ UNCHANGED <<Tree, stableContents, Journal, prepared, protected, Control, Environment>>
ClearEntry(id) ==
    /\ mode = "recovering" /\ id \in entries
    /\ contents[id] = baseline[id] /\ stableContents[id] = baseline[id]
    /\ (id \notin Range(before) \/ (shape = before /\ stableShape = before))
    /\ entries' = entries \ {id}
    /\ UNCHANGED <<Tree, Leaves, intent, stableIntent, before, baseline,
                    prepared, protected, Control, Environment, violations>>
DiscardUnpublished(id) ==
    /\ mode = "recovering" /\ id \in stages \intersect stableStages
    /\ id \notin Range(before)
    /\ (id \notin protected \/ Fault = "discard-published")
    /\ contents[id] = Absent /\ stableContents[id] = Absent
    /\ (id \notin foreignStageChildren \/ Fault = "discard-foreign")
    /\ stages' = stages \ {id} /\ stableStages' = stableStages \ {id}
    /\ prepared' = prepared \ {id}
    /\ UNCHANGED <<shape, stableShape, Leaves, Journal, protected, Control, Environment, violations>>
Restored == shape = before /\ stableShape = before /\ contents = baseline /\ stableContents = baseline
ClearEvolve ==
    /\ mode = "recovering" /\ intent /\ Restored /\ entries = {}
    /\ (stages \union stableStages) \subseteq protected
    /\ intent' = FALSE
    /\ UNCHANGED <<Tree, Leaves, stableIntent, entries, before, baseline,
                    prepared, protected, Control, Environment, violations>>
Finish ==
    /\ mode = "recovering" /\ ~intent /\ ~stableIntent /\ Restored /\ entries = {}
    /\ (stages \union stableStages) \subseteq protected
    /\ mode' = "done"
    /\ UNCHANGED <<Tree, Leaves, Journal, prepared, protected, target, round, recoveryRestarts, Environment, violations>>
ForeignChild(s) ==
    /\ mode \in {"writing", "recovering"} /\ s \notin foreignNodes
    /\ shape[s] # Absent /\ shape[s] = stableShape[s]
    /\ foreignNodes' = foreignNodes \union {s}
    /\ foreignAt' = [foreignAt EXCEPT ![s] = shape[s]]
    /\ UNCHANGED <<Tree, Leaves, Journal, prepared, protected, Control, foreignLeaves, foreignStageChildren, violations>>
ForeignLeaf(id) ==
    /\ mode \in {"writing", "recovering"} /\ Bound(id) /\ id \notin foreignLeaves
    /\ contents' = [contents EXCEPT ![id] = "foreign"]
    /\ stableContents' = [stableContents EXCEPT ![id] = "foreign"]
    /\ foreignLeaves' = foreignLeaves \union {id}
    /\ UNCHANGED <<Tree, Journal, prepared, protected, Control, foreignNodes, foreignAt, foreignStageChildren, violations>>
\* This marker follows the exact inode, not the stage pathname. Moving an
\* original container home preserves its foreign children without deletion.
ForeignStageChild(id) ==
    /\ mode \in {"writing", "recovering"} /\ id \in stages \intersect stableStages
    /\ id \notin foreignStageChildren
    /\ foreignStageChildren' = foreignStageChildren \union {id}
    /\ UNCHANGED <<Tree, Leaves, Journal, prepared, protected, Control,
                    foreignNodes, foreignAt, foreignLeaves, violations>>
\* This mutant deliberately grants directory deletion from borrowed ancestry.
ClearBorrowedForMutant ==
    /\ Fault = "borrowed-delete" /\ mode = "writing" /\ stableIntent
    /\ contents' = [contents EXCEPT !["borrowed-id"] = Absent]
    /\ stableContents' = [stableContents EXCEPT !["borrowed-id"] = Absent]
    /\ UNCHANGED <<Tree, Journal, prepared, protected, Control, Environment, violations>>
Next == RecordEvolve \/ SealIntent \/ SealTree \/ Publish \/ Commit \/ BeginHistoryRollback
        \/ StartRecovery \/ Crash(TRUE) \/ Crash(FALSE) \/ ClearEvolve \/ Finish \/ ClearBorrowedForMutant
        \/ (\E id \in Ids : Prepare(id) \/ RecordLeaf(id) \/ ClearChangedLeaf(id)
             \/ WriteDesiredLeaf(id) \/ SealLeaf(id) \/ RestoreLeaf(id) \/ ClearEntry(id)
             \/ DiscardUnpublished(id) \/ ForeignLeaf(id) \/ ForeignStageChild(id))
        \/ (\E s \in Slots : Retire(s) \/ Install(s) \/ ForeignChild(s))
        \/ UNCHANGED vars
Spec == Init /\ [][Next]_vars
TypeOK ==
    /\ shape \in Shapes /\ stableShape \in Shapes /\ before \in Shapes /\ target \in Shapes
    /\ stages \subseteq Ids /\ stableStages \subseteq Ids /\ prepared \subseteq Owned /\ protected \subseteq Owned
    /\ contents \in Contents /\ stableContents \in Contents /\ baseline \in Contents
    /\ intent \in BOOLEAN /\ stableIntent \in BOOLEAN /\ entries \subseteq Ids
    /\ mode \in {"writing", "recovering", "committed", "done"} /\ round \in 0..1
    /\ recoveryRestarts \in 0..2
    /\ foreignNodes \subseteq Slots /\ foreignAt \in Shapes /\ foreignLeaves \subseteq Ids
    /\ foreignStageChildren \subseteq Owned
    /\ violations \subseteq {"intent", "order", "leaf", "target-empty"}
MutationHasDurableEvolve == "intent" \notin violations
ChildBeforeParent == "order" \notin violations
OldLeavesAfterShapeRestore == "leaf" \notin violations
TargetInstallationEmpty == "target-empty" \notin violations
ForeignStageChildrenPreserved ==
    /\ foreignStageChildren \subseteq Range(shape) \union stages
    /\ foreignStageChildren \subseteq Range(stableShape) \union stableStages
HierarchyBound ==
    /\ \A id \in Range(shape) : Ancestors(id) \subseteq Range(shape)
    /\ \A id \in Range(stableShape) : Ancestors(id) \subseteq Range(stableShape)
BorrowedNeverRemoved == shape["borrowed"] = Initial["borrowed"]
                       /\ stableShape["borrowed"] = Initial["borrowed"]
ForeignChildrenPreserved == \A s \in foreignNodes : shape[s] = foreignAt[s] /\ stableShape[s] = foreignAt[s]
ForeignLeavesPreserved == \A id \in foreignLeaves : contents[id] = "foreign" /\ stableContents[id] = "foreign"
PublishedIdentitiesRetained ==
    /\ protected \subseteq Range(shape) \union stages
    /\ protected \subseteq Range(stableShape) \union stableStages
ExactRecovery == mode = "done" => Restored /\ entries = {} /\ ~intent /\ ~stableIntent
Safety == MutationHasDurableEvolve /\ ChildBeforeParent /\ OldLeavesAfterShapeRestore
          /\ HierarchyBound /\ BorrowedNeverRemoved /\ ForeignChildrenPreserved
          /\ PublishedIdentitiesRetained /\ ForeignLeavesPreserved /\ ExactRecovery
          /\ TargetInstallationEmpty /\ ForeignStageChildrenPreserved
NeverGrewNested == shape["nested"] = Absent
NeverRetargeted == shape["old"] # "replacement-id"
NeverRemovedOld == shape["old"] # Absent
NeverForeignSiblingCommit == ~(mode = "committed" /\ "root" \in foreignNodes)
NeverHistoryRollback == ~(mode = "committed" /\ round = 1)
NeverPublishedAbortStage == ~(mode = "done" /\ (protected \ Range(before)) \intersect stages # {})
NeverRootStaged == ~(shape["root"] = Absent /\ "root-id" \in stages)
NeverRepeatedEvolutionRecovery == recoveryRestarts < 2
NeverOriginalStageChildRestored == ~(mode = "recovering"
    /\ (foreignStageChildren \intersect Range(before) \intersect Range(shape)) # {})
=============================================================================
