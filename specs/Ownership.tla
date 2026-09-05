---- MODULE Ownership ----
(***************************************************************************)
(* The ownership-lineage model (plan/0029), alongside Transaction.tla.    *)
(*                                                                         *)
(* What the transaction protocol proves: a run's crash semantics. What   *)
(* THIS spec proves: which bytes gripsack may replace, and whether the   *)
(* pre-adoption origin survives the whole ownership epoch.               *)
(*                                                                         *)
(* The Rust harness (gripsack-exec/src/lineage_model.rs) drives the      *)
(* SHIPPED `plan_copy` decision function; this spec re-expresses the     *)
(* same algebra declaratively. A divergence between them is a finding.   *)
(*                                                                         *)
(* The lineage invariants are TRANSITION properties ("an apply must not *)
(* have written"), so the spec tracks the previous state and the last    *)
(* action explicitly, and the oracle reads them.                         *)
(***************************************************************************)
EXTENDS Integers

CONSTANTS C0, C1, C2, C3    \* contents: foreign original, repo A, repo B, user edit
NONE == "none"              \* model value (compares cleanly with records)

VARIABLES live,          \* what the destination holds
          desired,       \* what the repo wants
          manifest,      \* NONE or [hash: content, preserved: BOOL]
          origin,        \* NONE or the pre-adoption content
          declared,      \* the module is in the desired config
          lastAction,    \* transition tracking for the oracle
          prevLive, prevOrigin, prevDeclared, prevManifest

vars == <<live, desired, manifest, origin, declared,
          lastAction, prevLive, prevOrigin, prevDeclared, prevManifest>>

CONTENTS == {C0, C1, C2, C3}

TypeOK ==
    /\ live \in CONTENTS
    /\ desired \in {C1, C2}
    /\ manifest \in {NONE} \union [hash: CONTENTS, preserved: {TRUE, FALSE}]
    /\ origin \in {NONE} \union CONTENTS
    /\ declared \in {TRUE, FALSE}

\* The disposition decision — mirrors deploy.rs's plan_copy EXACTLY:
\* take-over first (adoption opens the epoch even when bytes match),
\* then satisfied, then managed update, else preserve. Preserved drift
\* NEVER authorizes.
PlanCopy(liveC, desiredC, prev, takeOver) ==
    IF takeOver THEN "takeover"
    ELSE IF liveC = desiredC THEN "satisfied"
    ELSE IF prev /= NONE /\ ~prev.preserved /\ liveC = prev.hash THEN "update"
    ELSE "preserve"

Snapshot(action) ==
    /\ lastAction' = action
    /\ prevLive' = live
    /\ prevOrigin' = origin
    /\ prevDeclared' = declared
    /\ prevManifest' = manifest

Init ==
    /\ live = C0            \* a foreign file exists (adoption story)
    /\ desired = C1
    /\ manifest = NONE
    /\ origin = NONE
    /\ declared = TRUE
    /\ lastAction = "init"
    /\ prevLive = C0
    /\ prevOrigin = NONE
    /\ prevDeclared = TRUE
    /\ prevManifest = NONE

SourceUpdate ==
    /\ desired' = IF desired = C1 THEN C2 ELSE C1
    /\ Snapshot("source-update")
    /\ UNCHANGED <<live, manifest, origin, declared>>

ExternalWrite ==
    /\ live' = C3           \* the app writes to its own config
    /\ Snapshot("external-write")
    /\ UNCHANGED <<desired, manifest, origin, declared>>

Apply ==
    /\ declared
    /\ LET plan == PlanCopy(live, desired, manifest, FALSE) IN
       CASE plan = "takeover" -> UNCHANGED <<live, manifest>>  \* dead: takeOver is FALSE here
        [] plan = "satisfied" ->
             /\ manifest' = [hash |-> desired, preserved |-> FALSE]
             /\ UNCHANGED live
        [] plan = "update" ->
             /\ live' = desired
             /\ manifest' = [hash |-> desired, preserved |-> FALSE]
        [] plan = "preserve" ->
             \* observed, never authority: the record is MARKED preserved
             /\ manifest' = [hash |-> live, preserved |-> TRUE]
             /\ UNCHANGED live
    /\ Snapshot("apply")
    /\ UNCHANGED <<desired, origin, declared>>

TakeOver ==
    /\ declared
    /\ origin' = IF origin = NONE THEN live ELSE origin  \* once per epoch
    /\ live' = desired
    /\ manifest' = [hash |-> desired, preserved |-> FALSE]
    /\ Snapshot("take-over")
    /\ UNCHANGED <<desired, declared>>

Undeclare ==
    /\ declared
    /\ LET managed == manifest /= NONE /\ ~manifest.preserved IN
       \* an epoch ends ONLY by restoring the origin; preserved drift
       \* was never ours and stays untouched
       /\ IF managed /\ origin /= NONE THEN live' = origin ELSE UNCHANGED live
    /\ origin' = NONE
    /\ manifest' = NONE
    /\ declared' = FALSE
    /\ Snapshot("undeclare")
    /\ UNCHANGED desired

Redeclare ==
    /\ ~declared
    /\ declared' = TRUE
    /\ Snapshot("redeclare")
    /\ UNCHANGED <<live, desired, manifest, origin>>

Next ==
    \/ SourceUpdate
    \/ ExternalWrite
    \/ Apply
    \/ TakeOver
    \/ Undeclare
    \/ Redeclare
    \/ (declared = FALSE /\ manifest = NONE /\ UNCHANGED vars)  \* quiesce

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* The lineage oracle (plan/0029):                                        *)
(*                                                                        *)
(*   An observed user value never becomes gripsack-authorized merely by  *)
(*   being observed, and an adopted origin remains recoverable until     *)
(*   gripsack successfully relinquishes ownership.                        *)
(***************************************************************************)

\* I1: an apply that followed a preserved-drift record must not have
\* written — unless the user converged to the desired content by hand.
DriftNeverPromoted ==
    lastAction = "apply" /\ prevManifest /= NONE /\ prevManifest.preserved
        /\ prevLive /= desired
        => live = prevLive

\* I2: the origin survives everything except a successful relinquish.
OriginSurvives ==
    prevOrigin /= NONE /\ lastAction /= "undeclare" => origin = prevOrigin

\* I3: relinquish restores the origin.
UndeclareRestores ==
    lastAction = "undeclare" /\ prevDeclared /\ prevManifest /= NONE
        /\ ~prevManifest.preserved /\ prevOrigin /= NONE
        => live = prevOrigin

\* I4 (state): preserved drift is always marked as such — the record
\* never shows user bytes as managed content.
ObservedIsMarked ==
    manifest /= NONE /\ manifest.hash = C3 => manifest.preserved

=============================================================================
