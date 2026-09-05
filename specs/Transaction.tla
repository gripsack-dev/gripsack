---- MODULE Transaction ----
(***************************************************************************)
(* The gripsack transaction protocol as a TLA+ specification (plan/0028).  *)
(*                                                                         *)
(* This is the LEARNING artifact and the independent second opinion: the   *)
(* CI-enforced proof lives in the Rust explorer (crates/gripsack-store/src *)
(* /journal/model.rs), which drives the shipped classify/decide functions. *)
(* This spec re-expresses the protocol in TLA+'s declarative style — if    *)
(* the two ever disagree, the disagreement is itself a finding.            *)
(*                                                                         *)
(* Reading guide:                                                          *)
(*   - VARIABLES are the machine's state. `volatile` is what the running   *)
(*     process has written; `durable` is what survived an fsync barrier;   *)
(*     `visible` is what a crash exposes — everything on a kill, durable   *)
(*     plus ANY SUBSET of pending writes on a power loss.                  *)
(*   - Actions ending in `'` assignments are the transitions. TLC walks    *)
(*     EVERY enabled action in EVERY reachable state.                      *)
(*   - `Oracle` is the invariant: plan/0020's sentence, formalized.        *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS PREV,       \* generation current points at when the run starts
          TARGET,     \* the generation this run builds (apply) or returns to
          OP,         \* "apply" | "rollback"
          KIND        \* "deploy" | "prune"


\* Abstract contents. One destination is enough — destinations are
\* journaled independently; the shared state is what we check.
CONSTANTS CPRIOR, CDEPLOYED, CEDITED
CONSTANT NONE               \* a model value: compares cleanly with records
ABSENT == "absent"          \* the destination does not exist
REMOVED == "removed"        \* the intended identity of a prune
CONTENTS == {CPRIOR, CDEPLOYED, CEDITED, ABSENT}

VARIABLES volatile, durable, visible, phase, step, edited, klass, beforeRecover

vars == <<volatile, durable, visible, phase, step, edited, klass, beforeRecover>>

NoEntry == NONE
NoMarker == NONE
NoCurrent == NONE

\* A disk: destination content, the current link, the journal.
DiskSpace ==
    [dest: CONTENTS,
     current: {PREV, TARGET, NoCurrent},
     entry: {NoEntry} \union [prior: CONTENTS, intended: CONTENTS \union {REMOVED}],
     marker: {NoMarker} \union [prev: {PREV, NONE}, target: {TARGET}]]

TypeOK ==
    /\ volatile \in DiskSpace
    /\ durable \in DiskSpace
    /\ visible \in DiskSpace
    /\ phase \in {"running", "crashed", "recovering", "done"}
    /\ step \in 0..6
    /\ edited \in {TRUE, FALSE}

\* The step sequence, [B] = fsync barrier:
\*   0. begin_run(prev, target)  [B]
\*   1. record(prior, intended) [B]   <- durable BEFORE the mutation
\*   2. mutate                    [B]
\*   3. flip: current := target  [B]   <- the commit point
\*   4. cleanup entry             [B]   <- barrier 1
\*   5. cleanup marker            [B]   <- barrier 2
StartContent == IF KIND = "deploy" THEN CPRIOR ELSE CDEPLOYED
Intended == IF KIND = "deploy" THEN CDEPLOYED ELSE REMOVED
AfterMutate == IF KIND = "deploy" THEN CDEPLOYED ELSE ABSENT

Effect(i, disk) ==
    CASE i = 0 -> [disk EXCEPT !.marker = [prev |-> PREV,
                                                 target |-> TARGET]]
      [] i = 1 -> [disk EXCEPT !.entry = [prior |-> disk.dest, intended |-> Intended]]
      [] i = 2 -> [disk EXCEPT !.dest = AfterMutate]
      [] i = 3 -> [disk EXCEPT !.current = TARGET]
      [] i = 4 -> [disk EXCEPT !.entry = NoEntry]
      [] i = 5 -> [disk EXCEPT !.marker = NoMarker]

Init ==
    /\ volatile = [dest |-> StartContent, current |-> PREV,
                   entry |-> NoEntry, marker |-> NoMarker]
    /\ durable = volatile
    /\ visible = volatile
    /\ phase = "running"
    /\ step = 0
    /\ edited = FALSE
    /\ klass = "none"
    /\ beforeRecover = volatile

\* A step completes: effect lands in volatile, then the barrier flushes.
DoStep ==
    /\ phase = "running"
    /\ step < 6
    /\ volatile' = Effect(step, volatile)
    /\ durable' = volatile'
    /\ step' = step + 1
    /\ phase' = IF step + 1 = 6 THEN "done" ELSE "running"
    /\ UNCHANGED <<visible, edited, klass, beforeRecover>>

\* A kill between steps: everything written persists; nothing new.
CrashKill ==
    /\ phase = "running"
    /\ visible' = volatile
    /\ beforeRecover' = volatile
    /\ phase' = "crashed"
    /\ UNCHANGED <<volatile, durable, step, edited, klass>>

\* A kill MID-STEP: the effect was written but the barrier never ran.
CrashMidStepKill ==
    /\ phase = "running"
    /\ step < 6
    /\ volatile' = Effect(step, volatile)
    /\ visible' = volatile'
    /\ beforeRecover' = volatile'
    /\ phase' = "crashed"
    /\ UNCHANGED <<durable, step, edited, klass>>

\* A power loss between steps: durable plus ANY subset of the pending
\* volatile writes — the disk is free to reorder.
CrashPower ==
    /\ phase = "running"
    /\ \E d \in {durable.dest, volatile.dest},
          c \in {durable.current, volatile.current},
          e \in {durable.entry, volatile.entry},
          m \in {durable.marker, volatile.marker} :
        /\ visible' = [dest |-> d, current |-> c, entry |-> e, marker |-> m]
        /\ beforeRecover' = visible'
    /\ phase' = "crashed"
    /\ UNCHANGED <<volatile, durable, step, edited, klass>>

CrashMidStepPower ==
    /\ phase = "running"
    /\ step < 6
    \* the effect is written but the barrier never ran: EVERY pending
    \* field (not just the step's own) may or may not persist
    /\ LET w == Effect(step, volatile) IN
       \E d \in {durable.dest, w.dest},
          c \in {durable.current, w.current},
          e \in {durable.entry, w.entry},
          m \in {durable.marker, w.marker} :
        /\ visible' = [dest |-> d, current |-> c, entry |-> e, marker |-> m]
        /\ beforeRecover' = visible'
        /\ volatile' = w
    /\ phase' = "crashed"
    /\ UNCHANGED <<durable, step, edited, klass>>

\* After the crash, the user may edit (or create) the destination.
MaybeUserEdit ==
    /\ phase = "crashed"
    /\ \/ /\ visible' = [visible EXCEPT !.dest = CEDITED]
          /\ edited' = TRUE
       \/ UNCHANGED <<visible, edited>>
    /\ beforeRecover' = visible'
    /\ phase' = "recovering"
    /\ UNCHANGED <<volatile, durable, step, klass>>

\* The decision logic, mirrored declaratively. The Rust explorer calls
\* the shipped functions; this spec re-expresses them — any divergence
\* between the two is a finding, not a tie-break.
Classify(prev, target, op, current) ==
    CASE prev /= NONE ->
            CASE current = target -> "committed"
              [] current = prev -> "uncommitted"
              [] OTHER -> "ambiguous"
      [] current = NoCurrent -> "uncommitted"
      [] op = "apply" /\ current >= target -> "committed"
      [] op = "rollback" /\ current <= target -> "committed"
      [] OTHER -> "uncommitted"

Decide(live, intended, prior) ==
    CASE live = intended -> "restore"
      [] live = prior -> "unchanged"
      [] live = ABSENT ->
            IF prior /= ABSENT THEN "restore" ELSE "unchanged"
      [] OTHER -> "keep"

Recover ==
    /\ phase = "recovering"
    /\ IF visible.marker = NoMarker /\ visible.entry = NoEntry
       THEN
         \* empty journal: recovery is a no-op
         /\ klass' = "none"
         /\ UNCHANGED visible
       ELSE
         LET m == visible.marker
             c == IF m = NoMarker
                  THEN "uncommitted"   \* entries without a marker
                  ELSE Classify(m.prev, m.target, OP, visible.current)
         IN
         /\ klass' = c
         /\ CASE c = "committed" ->
                  \* content stands; cleanup only
                  visible' = [visible EXCEPT !.entry = NoEntry, !.marker = NoMarker]
              [] c = "uncommitted" ->
                  \* entries restore per Decide; the journal ALWAYS
                  \* drains (zero-entry runs included — the marker is
                  \* stale by then)
                  LET e == visible.entry
                  IN
                  visible' = [visible EXCEPT
                      !.dest = IF e /= NoEntry /\ Decide(visible.dest, e.intended, e.prior) = "restore"
                               THEN e.prior ELSE @,
                      !.entry = NoEntry,
                      !.marker = NoMarker]
              [] OTHER -> UNCHANGED visible  \* ambiguous: change NOTHING
    /\ phase' = "done"
    /\ UNCHANGED <<volatile, durable, step, edited, beforeRecover>>

Next ==
    \/ DoStep
    \/ CrashKill
    \/ CrashMidStepKill
    \/ CrashPower
    \/ CrashMidStepPower
    \/ MaybeUserEdit
    \/ Recover
    \/ (phase = "done" /\ UNCHANGED vars)   \* terminal: stutter

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* The oracle: plan/0020's sentence. After ANY crash and recovery, the    *)
(* destination is the previous content, the committed target's content,   *)
(* or a post-crash edit that is KEPT — never an unexplained mixture —     *)
(* the journal drains, and ambiguous state changes nothing.               *)
(***************************************************************************)
Oracle ==
    phase = "done" =>
        CASE klass = "none" ->
                visible = beforeRecover    \* no journal: no recovery
          [] klass = "committed" ->
                /\ visible.marker = NoMarker
                /\ visible.entry = NoEntry
                /\ visible.current = TARGET
                /\ visible.dest = beforeRecover.dest  \* cleanup never writes
          [] klass = "uncommitted" ->
                /\ visible.marker = NoMarker
                /\ visible.entry = NoEntry
                /\ visible.current = PREV
                /\ IF edited
                   THEN visible.dest = CEDITED   \* user edits are kept
                   ELSE visible.dest = StartContent  \* the prior is restored
          [] OTHER ->  \* ambiguous: fail closed
                visible = beforeRecover

\* A clean run (no crash) commits and drains.
\* step only advances on DoStep, so step = 6 identifies a run that
\* never crashed; its final state lives in durable/visible's source.
CleanRunCommits ==
    (phase = "done" /\ step = 6) =>
        /\ durable.current = TARGET
        /\ durable.marker = NoMarker
        /\ durable.entry = NoEntry
        /\ durable.dest = IF KIND = "deploy" THEN CDEPLOYED ELSE ABSENT

=============================================================================
