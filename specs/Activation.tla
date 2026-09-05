---- MODULE Activation ----
(***************************************************************************)
(* Durable activation hooks (plan/0032) — the pending-record protocol     *)
(* that closes the last unrecorded crash window: post-activation          *)
(* adapters (cache refreshes, systemctl, custom hooks) used to run after  *)
(* the flip with no durable record, so a kill silently skipped them.      *)
(*                                                                        *)
(* The protocol under test:                                               *)
(*   1. begin      — write the pending record {generation, intents}       *)
(*                   BEFORE the flip (pre-flip is what closes the window; *)
(*                   the post-flip variant is the kept mutant below)      *)
(*   2. flip       — current := target (the run's commit point)           *)
(*   3. activate   — run the adapters (any crash here leaves pending)     *)
(*   4. clear      — adapters attempted; remove the pending record        *)
(*   5. crash      — kill at any boundary; the next run's resume step     *)
(*                   runs the intents iff pending names CURRENT, else     *)
(*                   discards the record                                  *)
(*                                                                        *)
(* Ordering guarantees are structural (action guards): BeginRun is       *)
(* disabled while a record is pending, so Resume always drains first;     *)
(* RunAdapters requires the flip of that same generation, so adapters    *)
(* can never run for a generation that isn't current. TLC checks the     *)
(* guards hold under EVERY interleaving with Crash.                       *)
(*                                                                        *)
(* THE invariant is NoSilentSkip: a committed generation is either        *)
(* activated or durably awaiting resume — the skip is unreachable.        *)
(*                                                                        *)
(* Not modeled: the no-intents case (no record is written at all —        *)
(* trivially safe); adapter FAILURE (warn-and-clear, 0001 §3.8 — a        *)
(* failed adapter does not retry; durability covers crashes, not          *)
(* poisoned hooks). Liveness is not model-checked: progress is one        *)
(* crash-free run, by construction.                                       *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS GENS,       \* generation numbers in play, e.g. {1, 2}
          NONE        \* a model value (compares cleanly with numbers)

VARIABLES current,    \* NONE | a generation — the committed state
          pending,    \* NONE | a generation whose intents await adapters
          activated,  \* the generations whose adapters have run
          phase       \* "idle" | "pending-written" | "flipped" | "ran-adapters"

vars == <<current, pending, activated, phase>>

TypeOK ==
    /\ current \in GENS \union {NONE}
    /\ pending \in GENS \union {NONE}
    /\ activated \in SUBSET GENS
    /\ phase \in {"idle", "pending-written", "flipped", "ran-adapters"}

Init ==
    /\ current = NONE
    /\ pending = NONE
    /\ activated = {}
    /\ phase = "idle"

\* --- the run -----------------------------------------------------------

\* Pre-flip: the pending record lands first (durable, atomic). A run
\* cannot begin while a record is pending — Resume drains it first.
BeginRun(g) ==
    /\ phase = "idle"
    /\ pending = NONE
    /\ pending' = g
    /\ phase' = "pending-written"
    /\ UNCHANGED <<current, activated>>

Flip(g) ==
    /\ phase = "pending-written"
    /\ pending = g
    /\ current' = g
    /\ phase' = "flipped"
    /\ UNCHANGED <<pending, activated>>

\* Adapters run; a crash during them leaves the record (Crash from
\* "flipped" or "ran-adapters") — re-running is the contract.
RunAdapters(g) ==
    /\ phase = "flipped"
    /\ pending = g
    /\ activated' = activated \union {g}
    /\ phase' = "ran-adapters"
    /\ UNCHANGED <<current, pending>>

ClearPending(g) ==
    /\ phase = "ran-adapters"
    /\ pending = g
    /\ pending' = NONE
    /\ phase' = "idle"
    /\ UNCHANGED <<current, activated>>

\* --- crash + resume ----------------------------------------------------

\* A kill anywhere: in-flight run state dies; durable records stand.
Crash ==
    /\ phase # "idle"
    /\ phase' = "idle"
    /\ UNCHANGED <<current, pending, activated>>

\* The next run's first act (after journal reconcile, before any
\* mutation): a pending record naming CURRENT re-runs its intents;
\* anything else is superseded or rolled back — discarded, never run.
Resume ==
    /\ phase = "idle"
    /\ pending # NONE
    /\ IF pending = current
       THEN activated' = activated \union {pending}
       ELSE activated' = activated
    /\ pending' = NONE
    /\ UNCHANGED <<current, phase>>

Next ==
    \/ \E g \in GENS : BeginRun(g) \/ Flip(g) \/ RunAdapters(g) \/ ClearPending(g)
    \/ Crash
    \/ Resume

Spec == Init /\ [][Next]_vars

\* --- the invariant -------------------------------------------------------

\* THE fix: a committed generation is either activated or durably
\* awaiting resume. ("Committed" = current points at it; idle = no run
\* in flight.)
NoSilentSkip ==
    \A g \in GENS :
        current = g /\ phase = "idle" => (g \in activated \/ pending = g)

=============================================================================
\* Mutant check (run by hand, recorded in plan/0032): move the pending
\* write AFTER Flip (BeginRun writes nothing; Flip writes the record) and
\* TLC finds NoSilentSkip violated in seconds — the crash between flip and
\* pending-write is exactly the pre-0.28 silent skip. That counterexample
\* is why the record is written pre-flip.
