---- MODULE ActivationIntentProofs ----
EXTENDS ActivationSteps

THEOREM SkipPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, Skip
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. Skip
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, Skip
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, Skip, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, Skip, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, Skip, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Skip, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, Skip, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, Skip, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM StartPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, Start
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. Start
  OBVIOUS
<1>3. TypeOK'
  <2>1. /\ TypeOK /\ active \in TXS /\ cursor \in Intents
        /\ stateC[active][cursor] \in States
    BY ONLY SMT, <1>1, <1>2, DeclaredIntentDomain
    DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Start
  <2>2. stateC[active][cursor].attempt + 1 \in Attempts
    BY ONLY SMT, <2>1, <1>2, ActivationParameters, OutcomeStateFields
    DEF Start, Attempts
  <2>3. State("started", stateC[active][cursor].attempt + 1) \in States
    BY AttemptOutcomeConstructor, <2>2
  <2>4. stateC' \in [TXS -> [Intents -> States]]
    BY ONLY SMT, <2>1, <2>3, <1>2, OutcomeSlotReplacementType DEF Start, TypeOK
  <2>5. QED
    BY ONLY SMT, <2>1, <2>4, <1>2, ActivationParameters
    DEF TypeOK, Start, Phases, home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, Start, CanStart
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, Start, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, <1>3, DirtyOutcomeRecords, OutcomeStateFields, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, DeclaredIntents, Intents,
      Start, CanStart, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Start, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, HistoryInvariant, ProcessInvariant, OutcomeAfterReturn, TerminalNoReplay,
      EffectsBindFullSelection, InvocationUsesDeclaredIntent, Start, CanStart
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, TypeOK, RecordInvariant, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits,
      Pairs, Triples, Intents, DeclaredIntents, AllSettled, Settled, Start, CanStart, home, records, archives,
      process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM StartBarrierPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, StartBarrier
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. StartBarrier
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain, OutcomeStateFields
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, StartBarrier
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, StartBarrier
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, StartBarrier, home, records, archives, process, history
<1>6. RecordInvariant'
  <2>1. /\ TypeOK /\ RecordInvariant /\ active \in TXS
    BY ONLY SMT, <1>1, <1>2
    DEF ActivationInductiveInvariant, ProcessInvariant, StartBarrier
  <2>2. Settled(active, stateC) \subseteq terminalEver
    BY ONLY SMT, <1>1, <1>2
    DEF ActivationInductiveInvariant, RecordInvariant, ProcessInvariant, StartBarrier, Settled, Terminal
  <2>3. QED
    BY ONLY SMT, <2>1, <2>2, <1>2, CorrectProtocol, OutcomeBarrierRecords
    DEF StartBarrier, history, home
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, ArchiveInvariant, ProcessInvariant, StartBarrier
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, HistoryInvariant, ProcessInvariant, OutcomeAfterReturn, TerminalNoReplay,
      EffectsBindFullSelection, InvocationUsesDeclaredIntent, StartBarrier
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, OptionalValueDomain, CorrectProtocol, DeclaredIntentDomain, OutcomeStateFields
  DEF ActivationInductiveInvariant, ProcessInvariant, TypeOK, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits,
      Pairs, Triples, Intents, DeclaredIntents, Attempts, AllSettled, Settled, StartBarrier, home, records, archives,
      process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM InvokePreservesInvariant ==
  ASSUME ActivationInductiveInvariant, Invoke
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. Invoke
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, Invoke
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, Invoke, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, Invoke, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, Invoke, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Invoke, home, records, archives, process, history
<1>8. HistoryInvariant'
  <2>1. /\ TypeOK /\ HistoryInvariant /\ active \in TXS /\ active # NONE
        /\ cursor \in DeclaredIntents(active) /\ currentC = active
        /\ Triple(active, cursor, permit.attempt) \in Triples
    BY ONLY SMT, <1>1, <1>2, ActivationParameters, DeclaredIntentDomain
    DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Invoke, IndexedPhases,
        NativePhases, PermitPhases, Permits, Triple, Triples
  <2>2. /\ invokedUnder'[Triple(active, cursor, permit.attempt)] = active
        /\ \A call \in Triples \ {Triple(active, cursor, permit.attempt)} :
             invokedUnder'[call] = invokedUnder[call]
    BY ONLY SMT, <2>1, <1>2, ReplacementFrame DEF Invoke, ReplaceCell, TypeOK
  <2>3. QED
    BY ONLY SMT, <2>1, <2>2, <1>2
    DEF TypeOK, HistoryInvariant, OutcomeAfterReturn, TerminalNoReplay, EffectsBindFullSelection,
        InvocationUsesDeclaredIntent, Invoke, Triple, records
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, TypeOK, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits,
      Pairs, Triples, Intents, DeclaredIntents, AllSettled, Settled, Invoke, home, records, archives, process,
      history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM ReturnPreservesInvariant ==
  ASSUME NEW verdict \in {"succeeded", "failed"}, ActivationInductiveInvariant, Return(verdict)
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant /\ verdict \in {"succeeded", "failed"}
  OBVIOUS
<1>2. Return(verdict)
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, Return
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, Return, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, Return, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, Return, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Return, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, HistoryInvariant, ProcessInvariant, OutcomeAfterReturn, TerminalNoReplay,
      EffectsBindFullSelection, InvocationUsesDeclaredIntent, Return
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, Return, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM WriteOutcomePreservesInvariant ==
  ASSUME ActivationInductiveInvariant, WriteOutcome
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. WriteOutcome
  OBVIOUS
<1>3. TypeOK'
  <2>1. /\ TypeOK /\ active \in TXS /\ cursor \in Intents
        /\ permit \in Permits /\ result \in {"succeeded", "failed"}
    BY ONLY SMT, <1>1, <1>2, DeclaredIntentDomain
    DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, IndexedPhases, WriteOutcome
  <2>2. State(result, permit.attempt) \in States
    BY ONLY SMT, <2>1, AttemptOutcomeConstructor DEF Permits
  <2>3. stateC' \in [TXS -> [Intents -> States]]
    BY ONLY SMT, <2>1, <2>2, <1>2, OutcomeSlotReplacementType DEF WriteOutcome, TypeOK
  <2>4. QED
    BY ONLY SMT, <2>1, <2>3, <1>2
    DEF TypeOK, WriteOutcome, Phases, home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, <1>3, OutcomeWritePlan, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, IndexedPhases, WriteOutcome,
      home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, WriteOutcome, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, <1>3, DirtyOutcomeRecords, OutcomeStateFields, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, DeclaredIntents, Intents,
      WriteOutcome, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, WriteOutcome, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, <1>3, OutcomeWriteHistory, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Triple, DeclaredIntents, Intents,
      WriteOutcome, home, records, archives, process, history
<1>9. ProcessInvariant'
  <2>1. /\ TypeOK /\ active \in TXS /\ cursor \in Intents
        /\ State(result, permit.attempt) \in States
    <3>1. /\ TypeOK /\ active \in TXS /\ cursor \in Intents
          /\ permit \in Permits /\ result \in {"succeeded", "failed"}
      BY ONLY SMT, <1>1, <1>2, DeclaredIntentDomain
      DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, IndexedPhases, WriteOutcome
    <3>2. State(result, permit.attempt) \in States
      BY ONLY SMT, <3>1, AttemptOutcomeConstructor DEF Permits
    <3>3. QED BY <3>1, <3>2
  <2>2. /\ stateC'[active][cursor] = State(result, permit.attempt)
        /\ \A other \in Intents \ {cursor} : stateC'[active][other] = stateC[active][other]
    BY ONLY SMT, <2>1, <1>2, OutcomeSlotReplacementFrame DEF TypeOK, WriteOutcome
  <2>3. QED
    BY ONLY SMT, <1>1, <1>2, <2>2
    DEF ActivationInductiveInvariant, ProcessInvariant, WriteOutcome, State, Phases, NativePhases,
        PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases,
        home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM OutcomeBarrierPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, OutcomeBarrier
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. OutcomeBarrier
  OBVIOUS
<1>3. TypeOK'
  <2>1. /\ TypeOK /\ active \in TXS /\ cursor \in Intents
    BY ONLY SMT, <1>1, <1>2, DeclaredIntentDomain
    DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, IndexedPhases, OutcomeBarrier
  <2>2. stateD' \in [TXS -> [Intents -> States]] /\ terminalEver' \subseteq Pairs
    <3>1. stateD' \in [TXS -> [Intents -> States]]
      BY ONLY SMT, <2>1, <1>2, ReplacementDomain DEF TypeOK, OutcomeBarrier, ReplaceCell
    <3>2. Settled(active, stateC) \subseteq Pairs
      BY SettledDomain, <2>1
    <3>3. terminalEver' \subseteq Pairs
      BY ONLY SMT, <2>1, <1>2, <3>2 DEF TypeOK, OutcomeBarrier
    <3>4. QED BY <3>1, <3>3
  <2>3. QED
    BY ONLY SMT, <2>1, <2>2, <1>2, ActivationParameters
    DEF TypeOK, OutcomeBarrier, Intents, Phases, home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealPlan
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, OutcomeBarrier, home, records, archives,
      process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, OutcomeBarrier, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeBarrierRecords
  DEF ActivationInductiveInvariant, ProcessInvariant, OutcomeBarrier, home, records, archives, process,
      history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealArchive
  DEF ActivationInductiveInvariant, ProcessInvariant, OutcomeBarrier, home, records, archives, process,
      history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealHistory
  DEF ActivationInductiveInvariant, ProcessInvariant, OutcomeBarrier, home, records, archives, process,
      history
<1>9. ProcessInvariant'
  <2>1. stateD'[active] = stateC[active]
    BY ONLY SMT, <1>1, <1>2, ReplacementFrame
    DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, OutcomeBarrier, ReplaceCell
  <2>2. QED
    BY ONLY SMT, <1>1, <1>2, <2>1
    DEF ActivationInductiveInvariant, ProcessInvariant, OutcomeBarrier, Phases, NativePhases,
        PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases,
        home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM SkipSupersededPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, SkipSuperseded
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. SkipSuperseded
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, SkipSuperseded, DeclaredIntents,
      Intents, home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, SkipSuperseded, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, SkipSuperseded, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, SkipSuperseded, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, SkipSuperseded, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, SkipSuperseded, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, SkipSuperseded, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM SupersedePreservesInvariant ==
  ASSUME ActivationInductiveInvariant, Supersede
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. Supersede
  OBVIOUS
<1>3. TypeOK'
  <2>1. /\ TypeOK /\ active \in TXS /\ cursor \in Intents
        /\ stateC[active][cursor] \in States
    BY ONLY SMT, <1>1, <1>2, DeclaredIntentDomain
    DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Supersede
  <2>2. State("superseded", stateC[active][cursor].attempt) \in States
    BY ONLY SMT, <2>1, OutcomeStateFields, SupersededOutcomeConstructor
  <2>3. stateC' \in [TXS -> [Intents -> States]]
    BY ONLY SMT, <2>1, <2>2, <1>2, OutcomeSlotReplacementType DEF Supersede, TypeOK
  <2>4. QED
    BY ONLY SMT, <2>1, <2>3, <1>2
    DEF TypeOK, Supersede, Phases, home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, <1>3, OutcomeWritePlan, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Supersede, home, records, archives,
      process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, Supersede, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, <1>3, DirtyOutcomeRecords, OutcomeStateFields, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, DeclaredIntents, Intents,
      Supersede, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Supersede, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, <1>3, OutcomeWriteHistory, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Triple, DeclaredIntents, Intents,
      Supersede, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, TypeOK, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits,
      Pairs, Triples, Intents, DeclaredIntents, AllSettled, Settled, Supersede, home, records, archives,
      process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM SupersedeBarrierPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, SupersedeBarrier
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. SupersedeBarrier
  OBVIOUS
<1>3. TypeOK'
  <2>1. /\ TypeOK /\ active \in TXS /\ cursor \in Intents
    BY ONLY SMT, <1>1, <1>2, DeclaredIntentDomain
    DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, IndexedPhases, SupersedeBarrier
  <2>2. stateD' \in [TXS -> [Intents -> States]] /\ terminalEver' \subseteq Pairs
    <3>1. stateD' \in [TXS -> [Intents -> States]]
      BY ONLY SMT, <2>1, <1>2, ReplacementDomain DEF TypeOK, SupersedeBarrier, ReplaceCell
    <3>2. Settled(active, stateC) \subseteq Pairs
      BY SettledDomain, <2>1
    <3>3. terminalEver' \subseteq Pairs
      BY ONLY SMT, <2>1, <1>2, <3>2 DEF TypeOK, SupersedeBarrier
    <3>4. QED BY <3>1, <3>3
  <2>3. QED
    BY ONLY SMT, <2>1, <2>2, <1>2, ActivationParameters
    DEF TypeOK, SupersedeBarrier, Intents, Phases, home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealPlan
  DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, SupersedeBarrier, home, records, archives,
      process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, SupersedeBarrier, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeBarrierRecords
  DEF ActivationInductiveInvariant, ProcessInvariant, SupersedeBarrier, home, records, archives, process,
      history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealArchive
  DEF ActivationInductiveInvariant, ProcessInvariant, SupersedeBarrier, home, records, archives, process,
      history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealHistory
  DEF ActivationInductiveInvariant, ProcessInvariant, SupersedeBarrier, home, records, archives, process,
      history
<1>9. ProcessInvariant'
  <2>1. stateD'[active] = stateC[active]
    BY ONLY SMT, <1>1, <1>2, ReplacementFrame
    DEF ActivationInductiveInvariant, TypeOK, ProcessInvariant, SupersedeBarrier, ReplaceCell
  <2>2. QED
    BY ONLY SMT, <1>1, <1>2, <2>1
    DEF ActivationInductiveInvariant, ProcessInvariant, SupersedeBarrier, Phases, NativePhases,
        PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases,
        home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM FinishSupersedingPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, FinishSuperseding
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. FinishSuperseding
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2
  DEF ActivationInductiveInvariant, TypeOK, FinishSuperseding, Phases,
      home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, FinishSuperseding, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, FinishSuperseding, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, FinishSuperseding, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, FinishSuperseding, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, FinishSuperseding, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, FinishSuperseding, home, records, archives, process,
      history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM FinishScanPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, FinishScan
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. FinishScan
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2
  DEF ActivationInductiveInvariant, TypeOK, FinishScan, Phases,
      home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, FinishScan, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, FinishScan, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, FinishScan, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, FinishScan, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, FinishScan, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, FinishScan, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

=============================================================================
