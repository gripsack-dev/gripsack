---- MODULE DirectoryHierarchy ----
\* Operational journal-v3 cutover explorer. Unlike a flat destination value,
\* a directory has an exact qualified identity, children and an untracked
\* occupant. Parent and leaf namespace writeback are independent of journal
\* writeback. Every action can be interrupted; no crash bound enters Spec.
\* This is the uncommitted/verification-failure recovery seam. Transaction's
\* exact commit-classification model and OwnedDirectory's induction remain
\* separate obligations, not consequences of a finite TLC run.
EXTENDS Naturals, FiniteSets
CONSTANTS LeafNames, OriginalTarget, OtherTarget, OwnedIdentity,
          ReplacementIdentity, JournalVersion, Fault, CrashBudget
ASSUME /\ IsFiniteSet(LeafNames)
       /\ OriginalTarget # OtherTarget
       /\ OwnedIdentity # ReplacementIdentity
       /\ JournalVersion \in {1, 2, 3}
       /\ CrashBudget \in Nat
\* Finite opaque qualified tokens: filesystem kind, fstatvfs FSID, inode,
\* required statx birth seconds and nanoseconds. Neither st_dev nor mount ID
\* is authority. These examples do not qualify a real filesystem observer.
OwnedInode == <<1, 101, 11, 1000, 1>>
OtherInode == <<1, 101, 12, 1000, 1>>
OtherFilesystem == <<1, 202, 11, 1000, 1>>
OtherBirth == <<1, 101, 11, 1000, 2>>
OtherFilesystemKind == <<2, 101, 11, 1000, 1>>
Original == [kind |-> "link", identity |-> OriginalTarget]
OtherLink == [kind |-> "link", identity |-> OtherTarget]
Owned == [kind |-> "directory", identity |-> OwnedIdentity]
Replacement == [kind |-> "directory", identity |-> ReplacementIdentity]
Missing == [kind |-> "absent"]
Parents == {Original, OtherLink, Owned, Replacement, Missing}
\* Optional observations are tagged records, not strings compared with records.
NoForeignParent == [kind |-> "unobserved"]
Values == {"absent", "first", "second", "foreign"}
NoEntry == [present |-> FALSE, before |-> "absent", after |-> "absent",
            ancestor |-> OwnedIdentity]
Entry(before, after) == [present |-> TRUE, before |-> before, after |-> after,
                         ancestor |-> OwnedIdentity]
Entries == [present: BOOLEAN, before: Values, after: Values,
            ancestor: {OwnedIdentity}]
Disk == [parent: Parents, leaves: [LeafNames -> Values],
         entries: [LeafNames -> Entries], extra: BOOLEAN,
         prepared: BOOLEAN, intent: BOOLEAN]
VARIABLES live, stable, mode, stage, foreignParent, foreignExtra,
          foreignLeaves, violations, crashes
vars == <<live, stable, mode, stage, foreignParent, foreignExtra,
          foreignLeaves, violations, crashes>>
Environment == <<foreignParent, foreignExtra, foreignLeaves>>
Faults == {"intent", "preparation", "ancestor", "order", "nonempty",
           "cleanup", "legacy", "target"}
Init ==
    /\ live = [parent |-> Original, leaves |-> [n \in LeafNames |-> "absent"],
                entries |-> [n \in LeafNames |-> NoEntry], extra |-> FALSE,
                prepared |-> FALSE, intent |-> FALSE]
    /\ stable = live
    /\ mode = "writing" /\ stage = 0
    /\ foreignParent = NoForeignParent /\ foreignExtra = FALSE
    /\ foreignLeaves = [n \in LeafNames |-> "none"]
    /\ violations = {} /\ crashes = 0

\* Preparation and the identity-bearing v3 intent each have their own barrier.
Prepare ==
    /\ mode = "writing" /\ stage = 0
    /\ Fault \notin {"missing-fsid", "missing-birth"}
    /\ live' = [live EXCEPT !.prepared = TRUE] /\ stage' = 1
    /\ UNCHANGED <<stable, mode, Environment, violations, crashes>>
SealPreparation ==
    /\ mode = "writing" /\ stage = 1
    /\ stable' = IF Fault = "preparation" THEN stable
                  ELSE [stable EXCEPT !.prepared = live.prepared]
    /\ stage' = 2
    /\ UNCHANGED <<live, mode, Environment, violations, crashes>>
RecordParent ==
    /\ mode = "writing" /\ stage = 2
    /\ (JournalVersion = 3 \/ Fault = "legacy")
    /\ live.parent = Original
    /\ live' = [live EXCEPT !.intent = TRUE] /\ stage' = 3
    /\ UNCHANGED <<stable, mode, Environment, violations, crashes>>
SealIntent ==
    /\ mode = "writing" /\ stage = 3
    /\ stable' = IF Fault = "intent" THEN stable
                  ELSE [stable EXCEPT !.intent = live.intent]
    /\ stage' = 4
    /\ UNCHANGED <<live, mode, Environment, violations, crashes>>
UnlinkParent ==
    /\ mode = "writing" /\ stage = 4
    /\ (live.parent = Original \/ Fault = "target")
    /\ live' = [live EXCEPT !.parent = Missing] /\ stage' = 5
    /\ violations' = violations
         \union (IF stable.intent THEN {} ELSE {"intent"})
         \union (IF stable.prepared THEN {} ELSE {"preparation"})
         \union (IF JournalVersion = 3 THEN {} ELSE {"legacy"})
         \union (IF live.parent = Original THEN {} ELSE {"target"})
    /\ UNCHANGED <<stable, mode, Environment, crashes>>
SealUnlink ==
    /\ mode = "writing" /\ stage = 5
    /\ stable' = [stable EXCEPT !.parent = live.parent] /\ stage' = 6
    /\ UNCHANGED <<live, mode, Environment, violations, crashes>>
InstallParent ==
    /\ mode = "writing" /\ stage = 6 /\ live.parent = Missing
    /\ live' = [live EXCEPT !.parent = Owned] /\ stage' = 7
    /\ UNCHANGED <<stable, mode, Environment, violations, crashes>>
SealInstalled ==
    /\ mode = "writing" /\ stage = 7
    /\ stable' = [stable EXCEPT !.parent = live.parent] /\ stage' = 8
    /\ UNCHANGED <<live, mode, Environment, violations, crashes>>
RecordLeaf(n) ==
    /\ mode = "writing" /\ stage = 8 /\ live.parent = Owned
    /\ live.entries[n] = stable.entries[n]
    /\ live.leaves[n] = stable.leaves[n]
    /\ live.leaves[n] \in {"absent", "first"}
    /\ (~live.entries[n].present \/ live.leaves[n] = live.entries[n].after)
    /\ live' = [live EXCEPT !.entries[n] = Entry(live.leaves[n],
                         IF live.leaves[n] = "absent" THEN "first" ELSE "second")]
    /\ UNCHANGED <<stable, mode, stage, Environment, violations, crashes>>
SealEntry(n) ==
    /\ live.entries[n] # stable.entries[n]
    /\ stable' = [stable EXCEPT !.entries[n] = live.entries[n]]
    /\ UNCHANGED <<live, mode, stage, Environment, violations, crashes>>
WriteLeaf(n) ==
    /\ mode = "writing" /\ stage = 8 /\ live.parent = Owned
    /\ live.entries[n].present /\ live.entries[n] = stable.entries[n]
    /\ live.leaves[n] = live.entries[n].before
    /\ live' = [live EXCEPT !.leaves[n] = live.entries[n].after]
    /\ UNCHANGED <<stable, mode, stage, Environment, violations, crashes>>
SealLeaf(n) ==
    /\ live.leaves[n] # stable.leaves[n]
    /\ stable' = [stable EXCEPT !.leaves[n] = live.leaves[n]]
    /\ UNCHANGED <<live, mode, stage, Environment, violations, crashes>>
\* Any dirty field can reach storage before a crash. These writebacks do not
\* acknowledge an omitted intent/preparation barrier to the running writer.
WritebackParent ==
    /\ live.parent # stable.parent
    /\ stable' = [stable EXCEPT !.parent = live.parent]
    /\ UNCHANGED <<live, mode, stage, Environment, violations, crashes>>
WritebackIntent ==
    /\ live.intent # stable.intent
    /\ stable' = [stable EXCEPT !.intent = live.intent]
    /\ UNCHANGED <<live, mode, stage, Environment, violations, crashes>>
WritebackPreparation ==
    /\ live.prepared # stable.prepared
    /\ stable' = [stable EXCEPT !.prepared = live.prepared]
    /\ UNCHANGED <<live, mode, stage, Environment, violations, crashes>>
Crash(power) ==
    /\ mode # "done"
    /\ live' = IF power THEN stable ELSE live
    /\ mode' = "recovering" /\ stage' = 0
    /\ UNCHANGED <<stable, Environment, violations, crashes>>
StartRecovery ==
    /\ mode = "writing" /\ mode' = "recovering" /\ stage' = 0
    /\ UNCHANGED <<live, stable, Environment, violations, crashes>>
\* Retargeting the predecessor between durable intent and unlink is a real
\* boundary; matching merely the fact that it is a symlink is insufficient.
Retarget ==
    /\ mode = "writing" /\ stage = 4 /\ foreignParent = NoForeignParent
    /\ live' = [live EXCEPT !.parent = OtherLink]
    /\ stable' = [stable EXCEPT !.parent = OtherLink]
    /\ foreignParent' = OtherLink
    /\ UNCHANGED <<mode, stage, foreignExtra, foreignLeaves, violations, crashes>>
ReplaceParent ==
    /\ mode = "recovering" /\ live.parent = Owned /\ foreignParent = NoForeignParent
    /\ live' = [live EXCEPT !.parent = Replacement,
                            !.leaves = [n \in LeafNames |-> "first"]]
    /\ stable' = [stable EXCEPT !.parent = Replacement,
                                !.leaves = [n \in LeafNames |-> "first"]]
    /\ foreignParent' = Replacement
    /\ foreignLeaves' = [n \in LeafNames |-> "first"]
    /\ UNCHANGED <<mode, stage, foreignExtra, violations, crashes>>
AddForeignChild ==
    /\ mode = "recovering" /\ live.parent = Owned /\ stable.parent = Owned
    /\ ~foreignExtra
    /\ live' = [live EXCEPT !.extra = TRUE]
    /\ stable' = [stable EXCEPT !.extra = TRUE] /\ foreignExtra' = TRUE
    /\ UNCHANGED <<mode, stage, foreignParent, foreignLeaves, violations, crashes>>
EditLeaf(n) ==
    /\ mode = "recovering" /\ live.parent = Owned /\ stable.parent = Owned
    /\ foreignLeaves[n] = "none"
    /\ live' = [live EXCEPT !.leaves[n] = "foreign"]
    /\ stable' = [stable EXCEPT !.leaves[n] = "foreign"]
    /\ foreignLeaves' = [foreignLeaves EXCEPT ![n] = "foreign"]
    /\ UNCHANGED <<mode, stage, foreignParent, foreignExtra, violations, crashes>>
MatchesAncestor(n) ==
    \/ live.parent = Owned /\ live.entries[n].ancestor = OwnedIdentity
    \/ Fault = "ancestor"
    \/ IF Fault = "inode-only" /\ live.parent.kind = "directory"
       THEN live.parent.identity[3] = live.entries[n].ancestor[3]
       ELSE FALSE
    \/ IF Fault = "birth-ignored" /\ live.parent.kind = "directory"
       THEN \A i \in 1..3 : live.parent.identity[i] = live.entries[n].ancestor[i]
       ELSE FALSE
    \/ IF Fault = "filesystem-kind-ignored" /\ live.parent.kind = "directory"
       THEN \A i \in 2..5 : live.parent.identity[i] = live.entries[n].ancestor[i]
       ELSE FALSE
RestoreLeaf(n) ==
    /\ mode = "recovering" /\ live.entries[n].present
    /\ MatchesAncestor(n)
    /\ live.leaves[n] \in {live.entries[n].after, live.entries[n].before}
    /\ live.leaves[n] # "absent"
    /\ live' = [live EXCEPT !.leaves[n] = "absent"]
    /\ violations' = violations \union (IF live.parent = Owned THEN {} ELSE {"ancestor"})
    /\ UNCHANGED <<stable, mode, stage, Environment, crashes>>
ClearLeaf(n) ==
    /\ mode = "recovering" /\ live.entries[n].present
    /\ live.leaves[n] = "absent"
    /\ (stable.leaves[n] = "absent" \/ Fault = "cleanup")
    /\ live' = [live EXCEPT !.entries[n] = NoEntry]
    /\ violations' = violations \union
          (IF stable.leaves[n] = "absent" THEN {} ELSE {"cleanup"})
    /\ UNCHANGED <<stable, mode, stage, Environment, crashes>>
ChildrenRestored == \A n \in LeafNames :
    /\ live.leaves[n] = "absent" /\ stable.leaves[n] = "absent"
    /\ ~live.entries[n].present /\ ~stable.entries[n].present
RemoveDirectory ==
    /\ mode = "recovering" /\ live.intent /\ live.parent = Owned
    /\ (ChildrenRestored \/ Fault = "order")
    /\ (~live.extra \/ Fault \in {"nonempty", "order"})
    /\ live' = [live EXCEPT !.parent = Missing,
          !.leaves = [n \in LeafNames |-> "absent"], !.extra = FALSE]
    /\ violations' = violations
          \union (IF ChildrenRestored THEN {} ELSE {"order"})
          \union (IF ~live.extra THEN {} ELSE {"nonempty"})
    /\ UNCHANGED <<stable, mode, stage, Environment, crashes>>
RestoreLink ==
    /\ mode = "recovering" /\ live.intent /\ live.parent = Missing
    /\ live' = [live EXCEPT !.parent = Original]
    /\ UNCHANGED <<stable, mode, stage, Environment, violations, crashes>>
ClearParent ==
    /\ mode = "recovering" /\ live.intent
    /\ live.parent = Original /\ stable.parent = Original /\ ChildrenRestored
    /\ live' = [live EXCEPT !.intent = FALSE]
    /\ UNCHANGED <<stable, mode, stage, Environment, violations, crashes>>
Finish ==
    /\ mode = "recovering" /\ ~live.intent /\ ~stable.intent
    /\ ChildrenRestored /\ live.parent = Original /\ stable.parent = Original
    /\ mode' = "done"
    /\ UNCHANGED <<live, stable, stage, Environment, violations, crashes>>
RecoveryWork == (\E n \in LeafNames : RestoreLeaf(n) \/ ClearLeaf(n))
                \/ RemoveDirectory \/ RestoreLink \/ ClearParent \/ Finish
Writeback == WritebackParent \/ WritebackIntent \/ WritebackPreparation
             \/ (\E n \in LeafNames : SealEntry(n) \/ SealLeaf(n))
Writer == Prepare \/ SealPreparation \/ RecordParent \/ SealIntent
          \/ UnlinkParent \/ SealUnlink \/ InstallParent \/ SealInstalled
          \/ (\E n \in LeafNames : RecordLeaf(n) \/ WriteLeaf(n))
External == Retarget \/ ReplaceParent \/ AddForeignChild
            \/ (\E n \in LeafNames : EditLeaf(n))
Next == Writer \/ Writeback \/ RecoveryWork \/ External \/ StartRecovery
        \/ Crash(TRUE) \/ Crash(FALSE)
        \/ UNCHANGED vars
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ live \in Disk /\ stable \in Disk
    /\ mode \in {"writing", "recovering", "done"} /\ stage \in 0..8
    /\ foreignParent \in {NoForeignParent, OtherLink, Replacement}
    /\ foreignExtra \in BOOLEAN
    /\ foreignLeaves \in [LeafNames -> Values \union {"none"}]
    /\ violations \subseteq Faults /\ crashes \in 0..CrashBudget
MutationHasDurableIntent == "intent" \notin violations
PreparedIdentityBeforeMutation == "preparation" \notin violations
ExactPredecessor == "target" \notin violations
V3AuthorityOnly == "legacy" \notin violations
AncestorIdentityBound == "ancestor" \notin violations
ChildBeforeParent == "order" \notin violations
EmptyDirectoryOnly == "nonempty" \notin violations
RestorationBeforeCleanup == "cleanup" \notin violations
ForeignParentPreserved == foreignParent # NoForeignParent =>
    live.parent = foreignParent /\ stable.parent = foreignParent
ForeignChildrenPreserved == foreignExtra => live.extra /\ stable.extra
ForeignLeavesPreserved == \A n \in LeafNames : foreignLeaves[n] # "none" =>
    live.leaves[n] = foreignLeaves[n] /\ stable.leaves[n] = foreignLeaves[n]
CompletedExactly == mode = "done" =>
    /\ live.parent = Original /\ stable.parent = Original
    /\ ChildrenRestored /\ ~live.intent /\ ~stable.intent
ConflictRetainsEvidence ==
    (foreignParent = Replacement \/ foreignExtra
     \/ (\E n \in LeafNames : foreignLeaves[n] # "none")) =>
        live.intent /\ stable.intent
DirectoryHasDurableAuthority ==
    (live.parent = Owned \/ stable.parent = Owned) =>
        stable.intent /\ stable.prepared /\ JournalVersion = 3
Safety == MutationHasDurableIntent /\ PreparedIdentityBeforeMutation
          /\ ExactPredecessor /\ V3AuthorityOnly /\ AncestorIdentityBound
          /\ ChildBeforeParent /\ EmptyDirectoryOnly /\ RestorationBeforeCleanup
          /\ ForeignParentPreserved /\ ForeignChildrenPreserved
          /\ ForeignLeavesPreserved /\ CompletedExactly /\ ConflictRetainsEvidence
          /\ DirectoryHasDurableAuthority
NeverInstalled == live.parent # Owned
NeverRepeatedWrite == \A n \in LeafNames : live.leaves[n] # "second"
NeverInterruptedRecovery == crashes < CrashBudget

\* Completion is deliberately separate: finite crashes, no external edits,
\* finite requested work, and weak fairness of successful recovery/writeback.
\* This wrapper interrupts dirty recovery windows; Spec covers all crashes.
BoundedCrash(power) ==
    /\ crashes < CrashBudget
    /\ mode = "recovering" /\ live # stable
    /\ live' = IF power THEN stable ELSE live
    /\ mode' = "recovering" /\ stage' = 0
    /\ crashes' = crashes + 1
    /\ UNCHANGED <<stable, Environment, violations>>
Progress == Writeback \/ RecoveryWork \/ StartRecovery
CompletionNext == Writer \/ Progress \/ BoundedCrash(TRUE) \/ BoundedCrash(FALSE)
                  \/ UNCHANGED vars
FairSpec == Init /\ [][CompletionNext]_vars /\ WF_vars(Progress)
RecoveryCompletes == <> (mode = "done")
=============================================================================
