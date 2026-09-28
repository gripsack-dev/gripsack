---- MODULE ActivationInvariant ----
EXTENDS Activation

NativePhases == {"ready", "start-dirty", "permitted", "running", "returned", "outcome-dirty"}
PermitPhases == {"permitted", "running", "returned"}
IndexedPhases == {"start-dirty", "permitted", "running", "returned", "outcome-dirty", "supersede-dirty"}
CoherentPhases == Phases \ {"idle", "load-home", "load-outcomes", "start-dirty", "outcome-dirty", "supersede-dirty"}
LoadedPhases == {"pre-flip", "flip-dirty", "load-outcomes", "authorize", "supersede", "supersede-dirty",
                 "archive", "archive-dirty", "archived"} \union NativePhases
ArchivePhases == {"archive", "archive-dirty", "archived", "clear-dirty"}

PlanInvariant ==
    /\ PointerHasPlan
    /\ \A t \in TXS : plans[t] = NONE =>
          /\ stateC[t] = [i \in Intents |-> Pending]
          /\ stateD[t] = [i \in Intents |-> Pending]
          /\ t \notin archiveC /\ t \notin archiveD
    /\ \A t \in TXS : \A i \in Intents \ DeclaredIntents(t) :
          stateC[t][i] = Pending /\ stateD[t][i] = Pending
HomeInvariant ==
    /\ \A current \in {currentC, currentD} :
          current # NONE /\ current \notin archiveD => pendingC = current /\ pendingD = current
    /\ (pendingC # pendingD /\ pendingD # NONE => pendingD \in archiveD)
    /\ NoSilentSkip
RecordInvariant ==
    /\ \A t \in TXS, i \in Intents :
          /\ stateD[t][i].attempt <= stateC[t][i].attempt
          /\ (Terminal(stateD[t][i]) => stateC[t][i] = stateD[t][i])
          /\ (Pair(t, i) \in terminalEver <=> Terminal(stateD[t][i]))
    /\ \A t \in TXS : t # pendingC => stateC[t] = stateD[t]
    /\ \A t \in TXS, i, j \in Intents :
          stateC[t][i] # stateD[t][i] /\ stateC[t][j] # stateD[t][j] => i = j
ArchiveInvariant ==
    /\ cleared \subseteq archiveD
    /\ \A t \in archiveC : AllSettled(t, stateD)
HistoryInvariant ==
    /\ OutcomeAfterReturn /\ TerminalNoReplay /\ EffectsBindFullSelection /\ InvocationUsesDeclaredIntent
    /\ \A call \in returned : invokedUnder[call] # NONE
ProcessInvariant ==
    /\ (phase = "idle" => active = NONE /\ cursor = 1 /\ permit = NONE /\ result = NONE)
    /\ (phase # "idle" => active \in TXS /\ plans[active] # NONE)
    /\ (phase \in IndexedPhases => cursor \in DeclaredIntents(active))
    /\ (phase \notin PermitPhases => permit = NONE)
    /\ (phase # "returned" => result = NONE)
    /\ (phase = "returned" => result \in {"succeeded", "failed"})
    /\ (phase \in CoherentPhases => stateC[active] = stateD[active])
    /\ (phase \in LoadedPhases => pendingC = active /\ pendingD = active)
    /\ (phase = "prepared" => pendingC = NONE)
    /\ (phase = "pointer-dirty" => pendingC = active)
    /\ (phase = "load-home" => pendingC = active)
    /\ (phase = "clear-dirty" => pendingC = NONE)
    /\ (phase \in NativePhases => currentC = active /\ currentD = active)
    /\ (phase \in {"load-outcomes", "authorize", "supersede", "supersede-dirty"} \union ArchivePhases => currentC = currentD)
    /\ (phase = "flip-dirty" => currentC = active)
    /\ (phase = "start-dirty" =>
          /\ stateC[active][cursor].kind = "started"
          /\ Pair(active, cursor) \notin terminalEver)
    /\ (phase \in {"start-dirty", "outcome-dirty", "supersede-dirty"} =>
          \A i \in Intents : i # cursor => stateC[active][i] = stateD[active][i])
    /\ (phase \in PermitPhases =>
          /\ permit # NONE /\ permit.tx = active /\ permit.intent = cursor
          /\ stateC[active][cursor] = State("started", permit.attempt)
          /\ stateD[active][cursor] = stateC[active][cursor])
    /\ (phase \in {"running", "returned"} =>
          invokedUnder[Triple(active, cursor, permit.attempt)] = active)
    /\ (phase = "returned" => Triple(active, cursor, permit.attempt) \in returned)
    /\ (phase = "outcome-dirty" => stateC[active][cursor].kind \in {"succeeded", "failed"})
    /\ (phase = "supersede-dirty" => stateC[active][cursor].kind = "superseded")
    /\ (phase \in ArchivePhases => AllSettled(active, stateD))
    /\ (phase \in {"archive-dirty", "archived", "clear-dirty"} => active \in archiveC)
    /\ (phase \in {"archived", "clear-dirty"} => active \in archiveD)

ActivationInductiveInvariant ==
    TypeOK /\ PlanInvariant /\ HomeInvariant /\ RecordInvariant /\ ArchiveInvariant
    /\ HistoryInvariant /\ ProcessInvariant

GcSafePendingResurrection == pendingC = NONE /\ pendingD # NONE =>
    pendingD \in archiveD /\ AllSettled(pendingD, stateD)
IndependentHomeResolution ==
    \A current \in {currentC, currentD}, pending \in {pendingC, pendingD} :
      current # NONE => pending = current \/ current \in archiveD

=============================================================================
