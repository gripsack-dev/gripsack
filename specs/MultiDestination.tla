---- MODULE MultiDestination ----
EXTENDS Integers, FiniteSets

CONSTANTS PREV, TARGET, KIND, CPRIOR, CDEPLOYED, CEDITED, NONE

\* Each transition writes at most one independently persistent disk field.
\* A mid-barrier power loss may therefore expose either version. Recovery
\* uses the same barriers and crash actions as the original writer.
Destinations == {1, 2}
Run == <<PREV, TARGET>>
ABSENT == "absent"
REMOVED == "removed"
CONTENTS == {CPRIOR, CDEPLOYED, CEDITED, ABSENT}
StartContent == IF KIND = "deploy" THEN CPRIOR ELSE CDEPLOYED
Intended == IF KIND = "deploy" THEN CDEPLOYED ELSE REMOVED
AfterMutate == IF KIND = "deploy" THEN CDEPLOYED ELSE ABSENT
NoEntry == NONE
NoMarker == NONE
EntrySpace == {NONE} \union [run: {Run}, prior: CONTENTS,
                              intended: CONTENTS \union {REMOVED}]
DiskSpace == [dest: [Destinations -> CONTENTS],
              current: {PREV, TARGET},
              entry: [Destinations -> EntrySpace],
              marker: {NONE} \union [run: {Run}, prev: {PREV}, target: {TARGET}]]

ASSUME /\ PREV \in Int /\ TARGET \in Int /\ PREV # TARGET
       /\ KIND \in {"deploy", "prune"}
       /\ Cardinality({CPRIOR, CDEPLOYED, CEDITED, ABSENT, REMOVED, NONE}) = 6

VARIABLES volatile, durable, visible, phase, step, edited, klass,
          beforeRecover, crashes
vars == <<volatile, durable, visible, phase, step, edited, klass,
          beforeRecover, crashes>>

TypeOK ==
    /\ volatile \in DiskSpace /\ durable \in DiskSpace
    /\ visible \in DiskSpace /\ beforeRecover \in DiskSpace
    /\ phase \in {"running", "crashed", "recovering", "done"}
    /\ step \in 0..8 /\ edited \in SUBSET Destinations
    /\ klass \in {"none", "committed", "uncommitted", "ambiguous"}
    /\ crashes \in 0..2

\* The shipped classifier (marker.rs), mirrored: exact equality only —
\* current == target commits, current == previous does not, anything
\* else is ambiguous and blocks. A generation flip is one atomic
\* rename, so current here can only ever be PREV or TARGET; the
\* fresh-machine (previous absent) and torn/foreign-current boundaries
\* cannot arise from a rename and are driven through the shipped
\* classify() by the Rust explorer instead. The historical direction
\* heuristic (apply with current >= target commits) was removed with
\* 0028's exact-commit decision; keeping it here left unreachable
\* branches that disagreed with production.
Classify(prev, target, current) ==
    CASE current = target -> "committed"
      [] current = prev -> "uncommitted"
      [] OTHER -> "ambiguous"

Decide(live, intended, prior) ==
    CASE live = intended -> "restore"
      [] live = prior -> "unchanged"
      [] live = ABSENT -> IF prior # ABSENT THEN "restore" ELSE "unchanged"
      [] OTHER -> "keep"

\* begin; independently record and mutate both destinations; publish;
\* independently delete both entries; finally delete the run marker.
Effect(i, d) ==
    CASE i = 0 -> [d EXCEPT !.marker = [run |-> Run, prev |-> PREV, target |-> TARGET]]
      [] i \in {1, 3} ->
           LET n == IF i = 1 THEN 1 ELSE 2 IN
           [d EXCEPT !.entry[n] = [run |-> Run, prior |-> d.dest[n], intended |-> Intended]]
      [] i \in {2, 4} ->
           [d EXCEPT !.dest[IF i = 2 THEN 1 ELSE 2] = AfterMutate]
      [] i = 5 -> [d EXCEPT !.current = TARGET]
      [] i \in {6, 7} -> [d EXCEPT !.entry[i - 5] = NONE]
      [] i = 8 -> [d EXCEPT !.marker = NONE]

\* Restoration and entry deletion are distinct barriers. In particular a
\* crash after restoration must safely replay the still-present entry.
RecoveryEffect(d, premature) ==
    IF \E n \in Destinations : d.entry[n] # NONE
    THEN LET n == CHOOSE j \in Destinations : d.entry[j] # NONE
             e == d.entry[n]
         IN IF ~premature /\
               Classify(PREV, TARGET, d.current) = "uncommitted" /\
               Decide(d.dest[n], e.intended, e.prior) = "restore"
            THEN [d EXCEPT !.dest[n] = e.prior]
            ELSE [d EXCEPT !.entry[n] = NONE]
    ELSE [d EXCEPT !.marker = NONE]

Init ==
    /\ volatile = [dest |-> [n \in Destinations |-> StartContent], current |-> PREV,
                   entry |-> [n \in Destinations |-> NONE], marker |-> NONE]
    /\ durable = volatile /\ visible = volatile /\ beforeRecover = volatile
    /\ phase = "running" /\ step = 0 /\ edited = {}
    /\ klass = "none" /\ crashes = 0

Empty(d) == d.marker = NONE /\ \A n \in Destinations : d.entry[n] = NONE
Work(premature) == IF phase = "running" THEN Effect(step, durable)
                  ELSE RecoveryEffect(durable, premature)

Advance(premature) ==
    /\ phase \in {"running", "recovering"}
    /\ volatile' = Work(premature) /\ durable' = volatile' /\ visible' = volatile'
    /\ phase' = IF (phase = "running" /\ step = 8) \/
                    (phase = "recovering" /\ Empty(durable'))
                 THEN "done" ELSE phase
    /\ step' = IF phase = "running" /\ step < 8 THEN step + 1 ELSE step
    /\ UNCHANGED <<edited, klass, beforeRecover, crashes>>

\* Includes kills between barriers, kills during a write, and power loss
\* during that write. No transition bundles unrelated destination writes.
Crash(premature) ==
    /\ phase \in {"running", "recovering"} /\ crashes < 2
    /\ \E d \in {durable, Work(premature)} :
          /\ durable' = d /\ visible' = d /\ volatile' = d
    /\ phase' = "crashed" /\ crashes' = crashes + 1
    /\ UNCHANGED <<step, edited, klass, beforeRecover>>

BeginRecover ==
    /\ phase = "crashed"
    /\ \E edits \in SUBSET Destinations :
          /\ edited' = edited \union edits
          /\ durable' = [durable EXCEPT !.dest =
                [n \in Destinations |-> IF n \in edits THEN CEDITED ELSE durable.dest[n]]]
    /\ visible' = durable' /\ volatile' = durable' /\ beforeRecover' = durable'
    /\ klass' = Classify(PREV, TARGET, durable'.current)
    /\ phase' = "recovering"
    /\ UNCHANGED <<step, crashes>>

Progress == Advance(FALSE) \/ BeginRecover
Next == Progress \/ Crash(FALSE)
Spec == Init /\ [][Next]_vars /\ WF_vars(Progress)
MutantNext == Advance(TRUE) \/ BeginRecover \/ Crash(TRUE)
PrematureCleanupSpec == Init /\ [][MutantNext]_vars

RunIdentity ==
    /\ durable.marker # NONE => durable.marker.run = Run
    /\ \A n \in Destinations : durable.entry[n] # NONE => durable.entry[n].run = Run
PreservedEdits == \A n \in edited : durable.dest[n] = CEDITED
RestoreBeforeCleanup ==
    durable.current = PREV =>
      \A n \in Destinations : durable.entry[n] = NONE =>
        durable.dest[n] \in {StartContent, CEDITED}
\* The classifier recovery actually branches on stays pinned to the
\* disk: with a marker present, committed <=> current flipped and
\* uncommitted <=> current still PREV. Ambiguity is unreachable here
\* by the atomic-rename argument above — the invariant says so rather
\* than leaving klass as unchecked bookkeeping.
RecoveryBranchSound ==
    (phase = "recovering" /\ durable.marker # NONE) =>
        /\ klass # "ambiguous"
        /\ (klass = "committed" <=> durable.current = TARGET)
        /\ (klass = "uncommitted" <=> durable.current = PREV)
Oracle ==
    phase = "done" =>
      /\ Empty(durable)
      /\ \A n \in Destinations :
           durable.dest[n] = IF n \in edited THEN CEDITED
                            ELSE IF durable.current = TARGET THEN AfterMutate ELSE StartContent
CleanRunCommits ==
    (phase = "done" /\ crashes = 0) =>
      /\ durable.current = TARGET /\ Empty(durable)
      /\ \A n \in Destinations : durable.dest[n] = AfterMutate
RecoveryCompletes == <> (phase = "done")
=============================================================================
