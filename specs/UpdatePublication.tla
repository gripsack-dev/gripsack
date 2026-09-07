-------------------------- MODULE UpdatePublication -------------------------
EXTENDS Naturals, TLC

(***************************************************************************
 Three already-started requests, two executable identities. old/new compete
 for the same capability-relative name; other has an independent lock. Each
 request has its own staging identity (the request key), never a shared temp.
 Initial snapshots deliberately precede every publication. Only the version
 observed UNDER the lock may decide whether to install.

 A write is a completed, bounded streamed copy, not a whole-binary allocation.
 Selection/source verification precedes this model. The modeled syscall order
 is write -> chmod -> file fsync -> atomic rename -> directory fsync. Each
 step may fail; no retry or automatic rollback follows a committed rename.
 Rename is an atomic namespace operation in one directory/filesystem; failure
 of rename here means no swap. A platform with ambiguous rename outcomes must
 resolve that ambiguity at its adapter boundary before using this model.

 durable records the last directory-fsync-acknowledged namespace. Failed
 directory fsync leaves durability unknown, represented conservatively by
 retaining the old acknowledged value, NOT by rolling the live namespace back.
 Successful file/dir fsync are assumed to honor their platform durability
 contracts. Power-loss recovery and adversarial external writers are outside
 this focused model. No claim is made that an unacknowledged swap is durable.

 Every syscall completes or returns failure under weak OS fairness. No wall
 clock timeout is promised for self-update. There are finitely many requests;
 lock holders fairly progress and release, so weak per-request fairness also
 suffices for eventual acquisition. Stuttering is explicit only at completion.
***************************************************************************)
CONSTANT Mutant
ASSUME Mutant \in {"none", "staleRecheck", "prematureRename", "rollback"}
Requests == {"old", "new", "other"}
Executables == {"core", "helper"}
Target(r) == IF r = "other" THEN "helper" ELSE "core"
Wanted(r) == IF r = "new" THEN 2 ELSE 1
Max(a, b) == IF a > b THEN a ELSE b
Phases == {"queued", "check", "write", "chmod", "filefsync", "rename",
           "dirfsync", "release", "done"}
Results == {"pending", "skipped", "preFailure", "postFailure", "installed"}
Image == [version : 0..2, written : BOOLEAN, mode : BOOLEAN, synced : BOOLEAN]
Base == [version |-> 0, written |-> TRUE, mode |-> TRUE, synced |-> TRUE]
Empty == [exists |-> FALSE, written |-> FALSE, mode |-> FALSE, synced |-> FALSE]
VARIABLES pc, lock, temp, visible, durable, high, before, checked, result,
          swapped, acknowledged
vars == <<pc, lock, temp, visible, durable, high, before, checked, result,
          swapped, acknowledged>>

Init ==
  /\ pc = [r \in Requests |-> "queued"]
  /\ lock = [e \in Executables |-> "none"]
  /\ temp = [r \in Requests |-> Empty]
  /\ visible = [e \in Executables |-> Base]
  /\ durable = visible
  /\ high = [e \in Executables |-> 0]
  /\ before = [r \in Requests |-> Base]
  /\ checked = [r \in Requests |-> 0]
  /\ result = [r \in Requests |-> "pending"]
  /\ swapped = [r \in Requests |-> FALSE]
  /\ acknowledged = [r \in Requests |-> FALSE]

TypeOK ==
  /\ pc \in [Requests -> Phases]
  /\ lock \in [Executables -> Requests \cup {"none"}]
  /\ temp \in [Requests -> [exists : BOOLEAN, written : BOOLEAN,
                             mode : BOOLEAN, synced : BOOLEAN]]
  /\ visible \in [Executables -> Image]
  /\ durable \in [Executables -> Image]
  /\ high \in [Executables -> 0..2]
  /\ before \in [Requests -> Image]
  /\ checked \in [Requests -> 0..2]
  /\ result \in [Requests -> Results]
  /\ swapped \in [Requests -> BOOLEAN]
  /\ acknowledged \in [Requests -> BOOLEAN]

Acquire(r) ==
  /\ pc[r] = "queued" /\ lock[Target(r)] = "none"
  /\ lock' = [lock EXCEPT ![Target(r)] = r]
  /\ pc' = [pc EXCEPT ![r] = "check"]
  /\ UNCHANGED <<temp, visible, durable, high, before, checked, result,
                  swapped, acknowledged>>
Recheck(r) ==
  LET v == IF Mutant = "staleRecheck" THEN 0 ELSE visible[Target(r)].version
  IN /\ pc[r] = "check"
     /\ checked' = [checked EXCEPT ![r] = v]
     /\ before' = [before EXCEPT ![r] = visible[Target(r)]]
     /\ pc' = [pc EXCEPT ![r] = IF Wanted(r) > v THEN "write" ELSE "release"]
     /\ result' = [result EXCEPT ![r] = IF Wanted(r) > v
                                      THEN "pending" ELSE "skipped"]
     /\ UNCHANGED <<lock, temp, visible, durable, high, swapped, acknowledged>>
Write(r) ==
  /\ pc[r] = "write"
  /\ temp' = [temp EXCEPT ![r] = [exists |-> TRUE, written |-> TRUE,
                                  mode |-> FALSE, synced |-> FALSE]]
  /\ pc' = [pc EXCEPT ![r] = "chmod"]
  /\ UNCHANGED <<lock, visible, durable, high, before, checked, result,
                  swapped, acknowledged>>
Chmod(r) ==
  /\ pc[r] = "chmod"
  /\ temp' = [temp EXCEPT ![r].mode = TRUE]
  /\ pc' = [pc EXCEPT ![r] = "filefsync"]
  /\ UNCHANGED <<lock, visible, durable, high, before, checked, result,
                  swapped, acknowledged>>
FileSync(r) ==
  /\ pc[r] = "filefsync"
  /\ temp' = [temp EXCEPT ![r].synced = TRUE]
  /\ pc' = [pc EXCEPT ![r] = "rename"]
  /\ UNCHANGED <<lock, visible, durable, high, before, checked, result,
                  swapped, acknowledged>>
Rename(r) ==
  /\ (pc[r] = "rename" \/ (Mutant = "prematureRename" /\ pc[r] = "chmod"))
  /\ visible' = [visible EXCEPT ![Target(r)] =
       [version |-> Wanted(r), written |-> temp[r].written,
        mode |-> temp[r].mode, synced |-> temp[r].synced]]
  /\ high' = [high EXCEPT ![Target(r)] = Max(@, Wanted(r))]
  /\ swapped' = [swapped EXCEPT ![r] = TRUE]
  /\ temp' = [temp EXCEPT ![r].exists = FALSE]
  /\ pc' = [pc EXCEPT ![r] = "dirfsync"]
  /\ UNCHANGED <<lock, durable, before, checked, result, acknowledged>>
DirSync(r) ==
  /\ pc[r] = "dirfsync"
  /\ durable' = [durable EXCEPT ![Target(r)] = visible[Target(r)]]
  /\ acknowledged' = [acknowledged EXCEPT ![r] = TRUE]
  /\ result' = [result EXCEPT ![r] = "installed"]
  /\ pc' = [pc EXCEPT ![r] = "release"]
  /\ UNCHANGED <<lock, temp, visible, high, before, checked, swapped>>

\* Includes a partially created staging file on write failure. RAII release
\* removes it; no namespace write is permitted on any pre-swap failure.
PreFailure(r) ==
  /\ pc[r] \in {"write", "chmod", "filefsync", "rename"}
  /\ temp' = [temp EXCEPT ![r].exists = TRUE]
  /\ result' = [result EXCEPT ![r] = "preFailure"]
  /\ pc' = [pc EXCEPT ![r] = "release"]
  /\ UNCHANGED <<lock, visible, durable, high, before, checked,
                  swapped, acknowledged>>
PostFailure(r) ==
  /\ pc[r] = "dirfsync"
  /\ visible' = IF Mutant = "rollback"
                  THEN [visible EXCEPT ![Target(r)] = before[r]] ELSE visible
  /\ result' = [result EXCEPT ![r] = "postFailure"]
  /\ pc' = [pc EXCEPT ![r] = "release"]
  /\ UNCHANGED <<lock, temp, durable, high, before, checked,
                  swapped, acknowledged>>
Release(r) ==
  /\ pc[r] = "release"
  /\ lock' = [lock EXCEPT ![Target(r)] = "none"]
  /\ temp' = [temp EXCEPT ![r] = Empty]
  /\ pc' = [pc EXCEPT ![r] = "done"]
  /\ UNCHANGED <<visible, durable, high, before, checked, result,
                  swapped, acknowledged>>
Step(r) == Acquire(r) \/ Recheck(r) \/ Write(r) \/ Chmod(r) \/ FileSync(r)
           \/ Rename(r) \/ DirSync(r) \/ PreFailure(r) \/ PostFailure(r)
           \/ Release(r)
AllDone == \A r \in Requests: pc[r] = "done"
Next == (\E r \in Requests: Step(r)) \/ (AllDone /\ UNCHANGED vars)
Spec == Init /\ [][Next]_vars /\ (\A r \in Requests: WF_vars(Step(r)))

LockDiscipline ==
  /\ \A r \in Requests:
       (pc[r] \notin {"queued", "done"}) <=> (lock[Target(r)] = r)
  /\ \A e \in Executables:
       lock[e] # "none" => Target(lock[e]) = e
PrivateStaging == \A r \in Requests:
  /\ (temp[r].exists => lock[Target(r)] = r)
  /\ (pc[r] = "done" => temp[r] = Empty)
Ready(i) == i.written /\ i.mode /\ i.synced
AtomicReady == \A e \in Executables: Ready(visible[e]) /\ Ready(durable[e])
NoDowngrade == \A e \in Executables: visible[e].version = high[e]
DurabilityOrder ==
  /\ \A e \in Executables: durable[e].version <= visible[e].version
  /\ \A r \in Requests:
       /\ (acknowledged[r] => swapped[r])
       /\ (result[r] = "installed" => acknowledged[r])
       /\ (swapped[r] => Wanted(r) > checked[r])
HonestResults == \A r \in Requests:
  /\ (result[r] \in {"skipped", "preFailure"} => ~swapped[r])
  /\ (result[r] = "preFailure" /\ pc[r] = "release"
       => visible[Target(r)] = before[r])
  /\ (result[r] = "postFailure" => swapped[r] /\ ~acknowledged[r])
  /\ (result[r] = "postFailure" /\ pc[r] = "release"
       => visible[Target(r)].version = Wanted(r))
  /\ (pc[r] = "done" => result[r] # "pending")
Terminates == <>AllDone
\* Non-vacuity witnesses: these are deliberately false safety assertions.
NeverPostFailure == \A r \in Requests: result[r] # "postFailure"
NeverIndependentLocks == ~(lock["core"] # "none" /\ lock["helper"] # "none")
NeverRecheckSkip == ~(pc["old"] = "done" /\ result["old"] = "skipped"
                       /\ visible["core"].version = 2)
=============================================================================
