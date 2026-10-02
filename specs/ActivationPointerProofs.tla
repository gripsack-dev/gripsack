---- MODULE ActivationPointerProofs ----
EXTENDS ActivationSteps

THEOREM PreparePreservesInvariant ==
  ASSUME NEW transaction \in TXS, NEW generation \in GENS, ActivationInductiveInvariant, Prepare(transaction, generation)
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant /\ transaction \in TXS /\ generation \in GENS
  OBVIOUS
<1>2. Prepare(transaction, generation)
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, Prepare
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, Prepare
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, Prepare, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, Prepare, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Prepare, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, Prepare, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, TypeOK, PlanInvariant, PointerHasPlan, Phases,
      NativePhases, PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal,
      Pair, Triple, Permits, Pairs, Triples, Intents, DeclaredIntents, AllSettled, Settled, Prepare, home,
      records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM WritePointerPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, WritePointer
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. WritePointer
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, WritePointer
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, WritePointer
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, WritePointer
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, ProcessInvariant, PlanInvariant, PointerHasPlan, WritePointer
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, WritePointer, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, WritePointer, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, WritePointer, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM SyncPointerPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, SyncPointer
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. SyncPointer
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, SyncPointer
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, SyncPointer
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, SyncPointer
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, SyncPointer, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, SyncPointer, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, SyncPointer, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, SyncPointer, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM FlipPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, Flip
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. Flip
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, Flip
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, Flip
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, Flip
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, Flip, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Flip, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, Flip, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, Flip, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM SyncFlipPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, SyncFlip
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. SyncFlip
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, SyncFlip
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, SyncFlip
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, SyncFlip
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, SyncFlip, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, SyncFlip, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, SyncFlip, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, SyncFlip, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM OpenPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, Open
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. Open
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, Open
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, Open, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, Open, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, Open, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Open, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, Open, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, TypeOK, PlanInvariant, PointerHasPlan, Phases,
      NativePhases, PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal,
      Pair, Triple, Permits, Pairs, Triples, Intents, DeclaredIntents, AllSettled, Settled, Open, home,
      records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM AuthorizePreservesInvariant ==
  ASSUME ActivationInductiveInvariant, Authorize
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. Authorize
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, Authorize, Matches
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, Authorize, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, Authorize, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, Authorize, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, Authorize, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, Authorize, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, Authorize, Matches, home, records, archives, process,
      history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM ClearPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, Clear
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. Clear
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, Clear
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, Clear
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, Clear
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, ProcessInvariant, PlanInvariant, PointerHasPlan, Clear
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, ArchiveInvariant, ProcessInvariant, Clear
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, Clear, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, Clear, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM ClearBarrierPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, ClearBarrier
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. ClearBarrier
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, ClearBarrier
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, ClearBarrier
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, ClearBarrier
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, ClearBarrier, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, ClearBarrier, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, ClearBarrier, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, ClearBarrier, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM SelectWithoutActivationPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, SelectWithoutActivation
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. SelectWithoutActivation
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, SelectWithoutActivation
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, SelectWithoutActivation
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, SelectWithoutActivation
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, SelectWithoutActivation, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, SelectWithoutActivation, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, SelectWithoutActivation, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, SelectWithoutActivation, home, records, archives, process,
      history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM SealWithoutActivationPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, SealWithoutActivation
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. SealWithoutActivation
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, SealWithoutActivation
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, SealWithoutActivation
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, SealWithoutActivation
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, SealWithoutActivation, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, SealWithoutActivation, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, SealWithoutActivation, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, SealWithoutActivation, home, records, archives, process,
      history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

=============================================================================
