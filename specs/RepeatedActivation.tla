---- MODULE RepeatedActivation ----
(***************************************************************************
 Repeated durable-activation runs against the SHIPPED record shapes
 (0032, tightened 0042): `activation.json` stores a GENERATION plus its
 intents, and `current` on disk is a generation number (Option<u64>).
 Neither record stores an invocation identifier. Two invocations that
 target the same generation are indistinguishable on disk, so the
 adapter obligation is generation-scoped: intents are idempotent
 refreshes by contract, and one attempt honors the obligation of
 whichever invocation wrote the live record.

 Resume's comparison is PHYSICAL generation equality — exactly
 `resume_pending`'s `Some(pending.generation) == current` — never
 equality of ghost invocation records. An earlier draft of this module
 compared [run, generation] pairs; that is stronger than the disk can
 express and would wrongly discard a record whose generation IS
 current. Consequence, deliberately reachable here: a run may target a
 generation that is ALREADY current (idempotent re-apply, rollback
 onto the current generation). Crashing after its record write but
 before the flip still leaves pending = current, so the next run's
 resume attempts those intents — that attempt is correct, not a skip.

 PHYSICAL variables (what survives a kill): current and pending.
 Ghost bookkeeping used only to STATE the invariant: owed (generations
 whose intents were durably declared by some record write) and
 attempted. A crash between an attempt and the clear re-attempts on
 the next run; a pending record naming a non-current generation is
 discarded, never run — both are the shipped resume step.

 Bound assumptions for tractable liveness: at most 3 runs and 2
 crashes, weak fairness on Progress. The finite run budget is a model
 checking assumption, not a claim that real work is finite. Not
 modeled: adapter failure (warn-and-clear, 0001 §3.8 — durability
 covers crashes, not poisoned hooks) and intent collection.
***************************************************************************)
EXTENDS Integers, FiniteSets
CONSTANTS GENS, NONE
ASSUME /\ GENS # {} /\ IsFiniteSet(GENS) /\ NONE \notin GENS

VARIABLES current, pending, owed, attempted, phase, runs, crashes
vars == <<current, pending, owed, attempted, phase, runs, crashes>>

TypeOK ==
    /\ current \in GENS \union {NONE} /\ pending \in GENS \union {NONE}
    /\ owed \in SUBSET GENS /\ attempted \in SUBSET GENS
    /\ attempted \subseteq owed
    /\ phase \in {"idle", "pending-written", "flipped", "ran-adapters"}
    /\ runs \in 0..3 /\ crashes \in 0..2

Init ==
    /\ current = NONE /\ pending = NONE
    /\ owed = {} /\ attempted = {}
    /\ phase = "idle" /\ runs = 3 /\ crashes = 0

\* The durable record is written pre-flip (0032): the write itself is
\* the declaration, so a fresh machine's first run and a re-run onto
\* the current generation both start from the same obligation.
BeginRun(g) ==
    /\ phase = "idle" /\ pending = NONE /\ runs > 0
    /\ pending' = g /\ owed' = owed \union {g}
    /\ runs' = runs - 1 /\ phase' = "pending-written"
    /\ UNCHANGED <<current, attempted, crashes>>

Flip(g) ==
    /\ phase = "pending-written" /\ pending = g
    /\ current' = g /\ phase' = "flipped"
    /\ UNCHANGED <<pending, owed, attempted, runs, crashes>>

RunAdapters(g) ==
    /\ phase = "flipped" /\ pending = g /\ current = g
    /\ attempted' = attempted \union {g} /\ phase' = "ran-adapters"
    /\ UNCHANGED <<current, pending, owed, runs, crashes>>

\* The next run's first act: a record naming the CURRENT generation
\* re-runs its intents, then still owes the clear (a crash between the
\* attempt and the clear re-attempts — idempotent).
ResumeAdapters ==
    /\ phase = "idle" /\ pending # NONE /\ pending = current
    /\ attempted' = attempted \union {pending} /\ phase' = "ran-adapters"
    /\ UNCHANGED <<current, pending, owed, runs, crashes>>

\* A record naming anything else (superseded, rolled back, or a
\* fresh machine that never flipped) is discarded, never run.
ResumeDiscard ==
    /\ phase = "idle" /\ pending # NONE /\ pending # current
    /\ pending' = NONE
    /\ UNCHANGED <<current, owed, attempted, phase, runs, crashes>>

ClearPending ==
    /\ phase = "ran-adapters" /\ pending # NONE
    /\ pending' = NONE /\ phase' = "idle"
    /\ UNCHANGED <<current, owed, attempted, runs, crashes>>

Crash ==
    /\ phase # "idle" /\ crashes < 2
    /\ phase' = "idle" /\ crashes' = crashes + 1
    /\ UNCHANGED <<current, pending, owed, attempted, runs>>

Progress ==
    \/ \E g \in GENS : BeginRun(g) \/ Flip(g) \/ RunAdapters(g)
    \/ ResumeAdapters \/ ResumeDiscard \/ ClearPending
Next == Progress \/ Crash
Spec == Init /\ [][Next]_vars /\ WF_vars(Progress)

\* An in-flight run always holds its durable record; the only way to
\* lose it is the calibrated mutant below.
InFlightHasRecord == phase # "idle" => pending # NONE

\* Adapters fire only for the record-named CURRENT generation —
\* resume_pending's guard, checked under every interleaving.
RunIdentity ==
    phase \in {"flipped", "ran-adapters"} =>
        pending # NONE /\ pending = current

\* THE safety obligation, generation-scoped like the disk records: a
\* current generation whose intents were durably declared is either
\* attempted or still durably awaiting resume.
NoSilentSkip ==
    \A g \in GENS :
        (current = g /\ g \in owed) => (g \in attempted \/ pending = g)

RecoveryCompletes == <>[](phase = "idle" /\ pending = NONE)

\* Calibrated negative: destroy the durable obligation at the crash
\* boundary after the flip, before the adapters. All types stay valid;
\* only NoSilentSkip falls.
LosePending ==
    /\ phase = "flipped" /\ crashes < 2
    /\ pending' = NONE /\ phase' = "idle" /\ crashes' = crashes + 1
    /\ UNCHANGED <<current, owed, attempted, runs>>
LostPendingSpec == Init /\ [][Next \/ LosePending]_vars
=============================================================================
