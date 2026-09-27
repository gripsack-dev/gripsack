---- MODULE Activation ----
(***************************************************************************
 R3 activation correspondence model, not the generalized M-V6 theorem.

 Current and pending pointers carry transaction identity, separate from the
 generation selected by that transaction. Immutable plans and initial Pending
 records are admitted/durable before a pointer can be published. Distinct
 instances retain separate state maps even when pointer cleanup is visible but
 not yet durable. There is no process-crash counter in this safety model.

 C is kernel-visible state; D is stable storage. ProcessDeath retains C.
 PowerLoss restores C from D. Independent Writeback actions allow dirty home,
 outcome-directory and archive writes to persist before an explicit barrier.
 Each visible atomic record publication has already synced its temporary file;
 directory sync covers that directory's published names. This abstracts the
 existing atomic_write_with_mode boundary, not an arbitrary write protocol.

 Native invocation/return are separate steps. Invocation records potential
 native effects, not remote success or complete-tree cleanup. A dead parent
 cannot infer a result; Started remains ambiguous. Receiver idempotency,
 process supervision, legacy migration, byte parsing and cryptographic hash
 injectivity are separate runtime/admission obligations. Id(t,i) abstracts an
 admitted stable instance/action/occurrence digest, with collision resistance
 assumed. No exactly-once or physical-storage certification is claimed.

 TLC cfgs supply finite transaction, generation, intent and attempt domains.
 MaxAttempts is the exploration bound, not the production u64 limit. Exhaustion
 retains pending evidence. RepeatedActivation adds a separately declared finite
 crash budget only for the pre-existing conditional completion experiment.
***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS TXS, GENS, IntentCount, MaxAttempts, NONE, MUTANT
ASSUME /\ TXS # {} /\ IsFiniteSet(TXS)
       /\ GENS # {} /\ IsFiniteSet(GENS)
       /\ NONE \notin TXS \cup GENS
       /\ IntentCount \in Nat \ {0} /\ MaxAttempts \in Nat \ {0}
       /\ MUTANT \in {"none", "unsealed_start", "early_success", "retry_failure",
                       "generation_commit", "early_clear", "generation_token"}

Intents == 1..IntentCount
Attempts == 1..MaxAttempts
Pair(t, i) == <<t, i>>
Triple(t, i, a) == <<t, i, a>>
State(k, a) == [kind |-> k, attempt |-> a]
Pending == State("pending", 0)
Terminal(s) == s.kind \in {"succeeded", "failed", "superseded"}
States == {Pending} \cup
          {State(k, a) : k \in {"started", "succeeded", "failed"}, a \in Attempts} \cup
          {State("superseded", a) : a \in 0..MaxAttempts}
Permits == [tx: TXS, intent: Intents, attempt: Attempts]
Phases == {"idle", "prepared", "pointer-dirty", "pre-flip", "flip-dirty",
           "load-home", "load-outcomes", "authorize", "ready", "start-dirty",
           "permitted", "running", "returned", "outcome-dirty", "supersede",
           "archive", "archive-dirty", "archived", "clear-dirty"}

VARIABLES currentC, currentD, pendingC, pendingD, plans,
          stateC, stateD, archiveC, archiveD,
          phase, active, cursor, permit, result,
          invokedUnder, returned, terminalEver, replayedTerminal, cleared

home == <<currentC, currentD, pendingC, pendingD>>
records == <<stateC, stateD>>
archives == <<archiveC, archiveD>>
process == <<phase, active, cursor, permit, result>>
history == <<invokedUnder, returned, terminalEver, replayedTerminal, cleared>>
vars == <<home, plans, records, archives, process, history>>

Pairs == TXS \X Intents
Triples == TXS \X Intents \X Attempts
Settled(t, states) == {Pair(t, i) : i \in {j \in Intents : Terminal(states[t][j])}}
AllSettled(t, states) == \A i \in Intents : Terminal(states[t][i])
Token(t, i) == IF MUTANT = "generation_token" THEN <<plans[t], i>> ELSE <<t, i>>

Init ==
    /\ currentC = NONE /\ currentD = NONE
    /\ pendingC = NONE /\ pendingD = NONE
    /\ plans = [t \in TXS |-> NONE]
    /\ stateC = [t \in TXS |-> [i \in Intents |-> Pending]]
    /\ stateD = stateC
    /\ archiveC = {} /\ archiveD = {}
    /\ phase = "idle" /\ active = NONE /\ cursor = 1
    /\ permit = NONE /\ result = NONE
    /\ invokedUnder = [call \in Triples |-> NONE]
    /\ returned = {} /\ terminalEver = {}
    /\ replayedTerminal = FALSE /\ cleared = {}

Prepare(t, g) ==
    /\ phase = "idle" /\ pendingC = NONE /\ plans[t] = NONE
    /\ plans' = [plans EXCEPT ![t] = g]
    /\ active' = t /\ cursor' = 1 /\ phase' = "prepared"
    /\ UNCHANGED <<home, records, archives, permit, result, history>>

WritePointer ==
    /\ phase = "prepared" /\ active # NONE
    /\ pendingC' = active /\ phase' = "pointer-dirty"
    /\ UNCHANGED <<currentC, currentD, pendingD, plans, records, archives,
                    active, cursor, permit, result, history>>

SyncPointer ==
    /\ phase = "pointer-dirty"
    /\ pendingD' = pendingC /\ currentD' = currentC /\ phase' = "pre-flip"
    /\ UNCHANGED <<currentC, pendingC, plans, records, archives,
                    active, cursor, permit, result, history>>

Flip ==
    /\ phase = "pre-flip" /\ pendingD = active
    /\ currentC' = active /\ phase' = "flip-dirty"
    /\ UNCHANGED <<currentD, pendingC, pendingD, plans, records, archives,
                    active, cursor, permit, result, history>>

SyncFlip ==
    /\ phase = "flip-dirty"
    /\ currentD' = currentC /\ pendingD' = pendingC /\ phase' = "ready"
    /\ UNCHANGED <<currentC, pendingC, plans, records, archives,
                    active, cursor, permit, result, history>>

Open ==
    /\ phase = "idle" /\ pendingC # NONE
    /\ active' = pendingC /\ cursor' = 1 /\ phase' = "load-home"
    /\ UNCHANGED <<home, plans, records, archives, permit, result, history>>

SealHome ==
    /\ phase = "load-home"
    /\ currentD' = currentC /\ pendingD' = pendingC /\ phase' = "load-outcomes"
    /\ UNCHANGED <<currentC, pendingC, plans, records, archives,
                    active, cursor, permit, result, history>>

SealOutcomes ==
    /\ phase = "load-outcomes"
    /\ stateD' = [stateD EXCEPT ![active] = stateC[active]]
    /\ archiveD' = IF active \in archiveC THEN archiveD \cup {active} ELSE archiveD
    /\ terminalEver' = terminalEver \cup Settled(active, stateC)
    /\ phase' = "authorize"
    /\ UNCHANGED <<home, plans, stateC, archiveC, active, cursor, permit, result,
                    invokedUnder, returned, replayedTerminal, cleared>>

Matches == currentC # NONE /\
    (IF MUTANT = "generation_commit" THEN plans[currentC] = plans[active]
     ELSE currentC = active)

Authorize ==
    /\ phase = "authorize"
    /\ phase' = IF Matches THEN "ready" ELSE "supersede"
    /\ UNCHANGED <<home, plans, records, archives, active, cursor, permit, result, history>>

Skip ==
    /\ phase = "ready" /\ cursor \in Intents
    /\ Terminal(stateC[active][cursor])
    /\ cursor' = cursor + 1
    /\ UNCHANGED <<home, plans, records, archives, phase, active, permit, result, history>>

CanStart(s) == s.kind \in {"pending", "started"} \/
              (MUTANT = "retry_failure" /\ s.kind = "failed")

Start ==
    /\ phase = "ready" /\ cursor \in Intents
    /\ CanStart(stateC[active][cursor])
    /\ stateC[active][cursor].attempt < MaxAttempts
    /\ stateC' = [stateC EXCEPT ![active][cursor] = State("started", @.attempt + 1)]
    /\ replayedTerminal' = (replayedTerminal \/ Pair(active, cursor) \in terminalEver)
    /\ phase' = "start-dirty"
    /\ UNCHANGED <<home, plans, stateD, archives, active, cursor, permit, result,
                    invokedUnder, returned, terminalEver, cleared>>

StartBarrier ==
    /\ phase = "start-dirty"
    /\ stateD' = IF MUTANT = "unsealed_start" THEN stateD
                  ELSE [stateD EXCEPT ![active] = stateC[active]]
    /\ permit' = [tx |-> active, intent |-> cursor, attempt |-> stateC[active][cursor].attempt]
    /\ phase' = "permitted"
    /\ UNCHANGED <<home, plans, stateC, archives, active, cursor, result, history>>

Invoke ==
    /\ phase = "permitted" /\ permit # NONE
    /\ invokedUnder' = [invokedUnder EXCEPT
           ![Triple(active, cursor, permit.attempt)] = currentC]
    /\ phase' = "running"
    /\ UNCHANGED <<home, plans, records, archives, active, cursor, permit, result,
                    returned, terminalEver, replayedTerminal, cleared>>

Return(verdict) ==
    /\ phase = "running" /\ permit # NONE
    /\ result' = verdict
    /\ returned' = returned \cup {Triple(active, cursor, permit.attempt)}
    /\ phase' = "returned"
    /\ UNCHANGED <<home, plans, records, archives, active, cursor, permit,
                    invokedUnder, terminalEver, replayedTerminal, cleared>>

WriteOutcome ==
    /\ phase = "returned" /\ permit # NONE
    /\ stateC' = [stateC EXCEPT ![active][cursor] = State(result, permit.attempt)]
    /\ permit' = NONE /\ result' = NONE /\ phase' = "outcome-dirty"
    /\ UNCHANGED <<home, plans, stateD, archives, active, cursor, history>>

OutcomeBarrier ==
    /\ phase = "outcome-dirty"
    /\ stateD' = [stateD EXCEPT ![active] = stateC[active]]
    /\ terminalEver' = terminalEver \cup Settled(active, stateC)
    /\ cursor' = cursor + 1 /\ phase' = "ready"
    /\ UNCHANGED <<home, plans, stateC, archives, active, permit, result,
                    invokedUnder, returned, replayedTerminal, cleared>>

Supersede ==
    /\ phase = "supersede"
    /\ stateC' = [stateC EXCEPT ![active] = [i \in Intents |->
          IF Terminal(stateC[active][i]) THEN stateC[active][i]
          ELSE State("superseded", stateC[active][i].attempt)]]
    /\ stateD' = [stateD EXCEPT ![active] = stateC'[active]]
    /\ terminalEver' = terminalEver \cup Settled(active, stateC')
    /\ phase' = "archive"
    /\ UNCHANGED <<home, plans, archives, active, cursor, permit, result,
                    invokedUnder, returned, replayedTerminal, cleared>>

FinishScan ==
    /\ phase = "ready" /\ cursor > IntentCount /\ AllSettled(active, stateC)
    /\ phase' = "archive"
    /\ UNCHANGED <<home, plans, records, archives, active, cursor, permit, result, history>>

WriteArchive ==
    /\ phase = "archive" /\ AllSettled(active, stateD)
    /\ archiveC' = archiveC \cup {active} /\ phase' = "archive-dirty"
    /\ UNCHANGED <<home, plans, records, archiveD, active, cursor, permit, result, history>>

ArchiveBarrier ==
    /\ phase = "archive-dirty"
    /\ archiveD' = archiveD \cup {active} /\ phase' = "archived"
    /\ UNCHANGED <<home, plans, records, archiveC, active, cursor, permit, result, history>>

Clear ==
    /\ active # NONE /\ pendingC = active
    /\ phase = "archived" \/ (MUTANT = "early_clear" /\ phase = "archive")
    /\ pendingC' = NONE /\ cleared' = cleared \cup {active} /\ phase' = "clear-dirty"
    /\ UNCHANGED <<currentC, currentD, pendingD, plans, records, archives,
                    active, cursor, permit, result, invokedUnder, returned, terminalEver, replayedTerminal>>

ClearBarrier ==
    /\ phase = "clear-dirty"
    /\ currentD' = currentC /\ pendingD' = pendingC
    /\ phase' = "idle" /\ active' = NONE /\ cursor' = 1
    /\ UNCHANGED <<currentC, pendingC, plans, records, archives, permit, result, history>>

EarlySuccess ==
    /\ MUTANT = "early_success" /\ phase = "permitted"
    /\ stateC' = [stateC EXCEPT ![active][cursor] = State("succeeded", permit.attempt)]
    /\ permit' = NONE /\ phase' = "outcome-dirty"
    /\ UNCHANGED <<home, plans, stateD, archives, active, cursor, result, history>>

WritebackHome ==
    /\ currentC # currentD \/ pendingC # pendingD
    /\ currentD' = currentC /\ pendingD' = pendingC
    /\ UNCHANGED <<currentC, pendingC, plans, records, archives, process, history>>

WritebackOutcomes(t) ==
    /\ stateC[t] # stateD[t]
    /\ stateD' = [stateD EXCEPT ![t] = stateC[t]]
    /\ terminalEver' = terminalEver \cup Settled(t, stateC)
    /\ UNCHANGED <<home, plans, stateC, archives, process,
                    invokedUnder, returned, replayedTerminal, cleared>>

WritebackArchive(t) ==
    /\ t \in archiveC /\ t \notin archiveD
    /\ archiveD' = archiveD \cup {t}
    /\ UNCHANGED <<home, plans, records, archiveC, process, history>>

ProcessDeath ==
    /\ phase # "idle"
    /\ phase' = "idle" /\ active' = NONE /\ cursor' = 1 /\ permit' = NONE /\ result' = NONE
    /\ UNCHANGED <<home, plans, records, archives, history>>

PowerLoss ==
    /\ (phase # "idle" \/ currentC # currentD \/ pendingC # pendingD
          \/ stateC # stateD \/ archiveC # archiveD)
    /\ currentC' = currentD /\ pendingC' = pendingD
    /\ stateC' = stateD /\ archiveC' = archiveD
    /\ phase' = "idle" /\ active' = NONE /\ cursor' = 1 /\ permit' = NONE /\ result' = NONE
    /\ UNCHANGED <<currentD, pendingD, plans, stateD, archiveD, history>>

Progress ==
    \/ \E t \in TXS, g \in GENS : Prepare(t, g)
    \/ WritePointer \/ SyncPointer \/ Flip \/ SyncFlip
    \/ Open \/ SealHome \/ SealOutcomes \/ Authorize \/ Skip \/ Start \/ StartBarrier
    \/ Invoke \/ (\E verdict \in {"succeeded", "failed"} : Return(verdict))
    \/ WriteOutcome \/ OutcomeBarrier \/ Supersede \/ FinishScan
    \/ WriteArchive \/ ArchiveBarrier \/ Clear \/ ClearBarrier \/ EarlySuccess
    \/ WritebackHome \/ (\E t \in TXS : WritebackOutcomes(t) \/ WritebackArchive(t))
Next == Progress \/ ProcessDeath \/ PowerLoss
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ currentC \in TXS \cup {NONE} /\ currentD \in TXS \cup {NONE}
    /\ pendingC \in TXS \cup {NONE} /\ pendingD \in TXS \cup {NONE}
    /\ plans \in [TXS -> GENS \cup {NONE}]
    /\ stateC \in [TXS -> [Intents -> States]] /\ stateD \in [TXS -> [Intents -> States]]
    /\ archiveC \subseteq TXS /\ archiveD \subseteq archiveC
    /\ phase \in Phases /\ active \in TXS \cup {NONE}
    /\ cursor \in 1..(IntentCount + 1) /\ permit \in Permits \cup {NONE}
    /\ result \in {"succeeded", "failed", NONE}
    /\ invokedUnder \in [Triples -> TXS \cup {NONE}] /\ returned \subseteq Triples
    /\ terminalEver \subseteq Pairs /\ replayedTerminal \in BOOLEAN /\ cleared \subseteq TXS

PointerHasPlan == \A pointer \in {currentC, currentD, pendingC, pendingD} :
                    pointer # NONE => plans[pointer] # NONE
PermitAfterDurableStart == permit # NONE =>
    stateD[permit.tx][permit.intent] = State("started", permit.attempt)
OutcomeAfterReturn == \A t \in TXS, i \in Intents, states \in {stateC, stateD} :
    states[t][i].kind \in {"succeeded", "failed"} => Triple(t, i, states[t][i].attempt) \in returned
TerminalNoReplay == ~replayedTerminal
EffectsBindFullSelection == \A call \in Triples : invokedUnder[call] # NONE => invokedUnder[call] = call[1]
ArchiveBeforeClear == cleared \subseteq archiveD
ArchiveHasTerminalOutcomes == \A t \in archiveD : AllSettled(t, stateD)
NoSilentSkip == currentD # NONE => (pendingD = currentD \/ currentD \in archiveD)
DistinctIntentIdentity == \A t, u \in TXS, i, j \in Intents :
    plans[t] # NONE /\ plans[u] # NONE /\ (t # u \/ i # j) => Token(t, i) # Token(u, j)
=============================================================================
