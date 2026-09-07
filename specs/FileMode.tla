------------------------------- MODULE FileMode -------------------------------
(***************************************************************************
 Independent permission policy (0043), not a proof about arbitrary Rust.
 Ownership models opaque content identities; Transaction assumes an intended
 identity is already correct. Neither chooses a template's landing policy.
 The Rust explorer exercises plan_entry_op and execute_op; e2e additionally
 exercises source-policy selection in deploy and preview.

 Scope: one destination, fixed source, mode-only user edits, repeated apply,
 prune, and one rollback to an earlier managed generation. Content changes,
 crash windows, source-execute deltas, and multi-module graphs have separate
 Rust/e2e/transaction coverage. A link carries its target mode: chmod of its
 immutable store target is store corruption, not destination drift.

 Modes are symbolic values. 0600 is PRIVATE DATA, never executable. Fresh
 whole-file outputs normalize source exec to 0755/0644. Existing merge hosts
 retain their full mode. Rollback is authorized only by an intact receipt;
 preserved observations never authorize overwriting or deleting user state.
***************************************************************************)
EXTENDS Integers, FiniteSets
CONSTANTS SURFACE, SRC, M_0644, M_0755, M_0600, NOFILE,
          FIXED_TEMPLATE, BYTES_ONLY
MODES == {M_0644, M_0755, M_0600}
ASSUME /\ SRC \in MODES
       /\ NOFILE \notin MODES
       /\ SURFACE \in {"copy", "template", "merge", "link"}

VARIABLES live, recorded, preserved, phase, initial, target,
          lastAction, prevLive, prevRecorded, prevPreserved
vars == <<live, recorded, preserved, phase, initial, target,
          lastAction, prevLive, prevRecorded, prevPreserved>>

CarriesExec(m) == m = M_0755
LandingMode ==
    IF SURFACE = "merge" THEN IF initial = NOFILE THEN M_0644 ELSE initial
    ELSE IF SURFACE = "link" THEN SRC
    ELSE IF SURFACE = "template" /\ FIXED_TEMPLATE THEN M_0644
    ELSE IF CarriesExec(SRC) THEN M_0755 ELSE M_0644
SameMode(a, b) == BYTES_ONLY \/ a = b
Satisfied == SameMode(live, LandingMode)
Intact == ~preserved /\ SameMode(live, recorded)
Snapshot(action) ==
    /\ lastAction' = action
    /\ prevLive' = live
    /\ prevRecorded' = recorded
    /\ prevPreserved' = preserved

Init ==
    /\ initial \in IF SURFACE = "merge" THEN MODES \union {NOFILE} ELSE {NOFILE}
    /\ target \in MODES
    /\ live = initial
    /\ recorded = NOFILE
    /\ preserved = FALSE
    /\ phase = "deploying"
    /\ lastAction = "init"
    /\ prevLive = NOFILE
    /\ prevRecorded = NOFILE
    /\ prevPreserved = FALSE

Deploy ==
    /\ phase = "deploying"
    /\ live' = LandingMode
    /\ recorded' = live'
    /\ preserved' = FALSE
    /\ phase' = "user"
    /\ Snapshot("deploy")
    /\ UNCHANGED <<initial, target>>

UserChmod(m) ==
    /\ phase = "user" /\ SURFACE # "link" /\ live # NOFILE
    /\ m \in MODES
    /\ live' = m
    /\ Snapshot("chmod")
    /\ UNCHANGED <<recorded, preserved, phase, initial, target>>

UserRemove ==
    /\ phase = "user" /\ live # NOFILE
    /\ live' = NOFILE
    /\ Snapshot("remove")
    /\ UNCHANGED <<recorded, preserved, phase, initial, target>>

Redeploy ==
    /\ phase = "user"
    /\ IF live = NOFILE
       THEN /\ live' = LandingMode /\ preserved' = FALSE
       ELSE /\ UNCHANGED live /\ preserved' = ~Satisfied
    /\ recorded' = live'
    /\ Snapshot("redeploy")
    /\ UNCHANGED <<phase, initial, target>>

Prune ==
    /\ phase = "user"
    /\ IF live = NOFILE \/ Intact THEN live' = NOFILE ELSE UNCHANGED live
    /\ phase' = "done"
    /\ Snapshot("prune")
    /\ UNCHANGED <<recorded, preserved, initial, target>>

Rollback ==
    /\ phase = "user"
    /\ IF live = NOFILE \/ Intact
       THEN /\ live' = IF SURFACE = "template" /\ FIXED_TEMPLATE THEN M_0644 ELSE target
            /\ preserved' = FALSE
       ELSE /\ UNCHANGED live /\ preserved' = TRUE
    /\ recorded' = live'
    /\ phase' = "done"
    /\ Snapshot("rollback")
    /\ UNCHANGED <<initial, target>>

Next == Deploy \/ (\E m \in MODES : UserChmod(m)) \/ UserRemove
        \/ Redeploy \/ Prune \/ Rollback
Spec == Init /\ [][Next]_vars

TypeOK == /\ live \in MODES \union {NOFILE}
          /\ recorded \in MODES \union {NOFILE}
          /\ preserved \in BOOLEAN
          /\ phase \in {"deploying", "user", "done"}

ExecSurvivesDeploy ==
    lastAction = "deploy" /\ SURFACE # "merge" /\ CarriesExec(SRC)
        => CarriesExec(live)

RecordNamesLive ==
    lastAction \in {"deploy", "redeploy", "rollback"} => recorded = live

ChmodIsDrift ==
    phase = "user" /\ live # NOFILE /\ live # LandingMode => ~Satisfied

PruneRespectsDrift ==
    lastAction = "prune" /\ prevLive # NOFILE
        /\ (prevPreserved \/ prevLive # prevRecorded) => live = prevLive

RollbackIsExact ==
    lastAction = "rollback" =>
        IF prevLive = NOFILE \/ (~prevPreserved /\ prevLive = prevRecorded)
        THEN live = target
        ELSE live = prevLive

DriftNeverPromoted ==
    lastAction = "redeploy" /\ prevLive # NOFILE /\ prevLive # LandingMode
        => /\ live = prevLive /\ preserved
=============================================================================
