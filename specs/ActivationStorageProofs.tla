---- MODULE ActivationStorageProofs ----
EXTENDS ActivationSteps

THEOREM SealOutcomesPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, SealOutcomes
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. SealOutcomes
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, SealOutcomes
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, SealOutcomes
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, SealOutcomes
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeBarrierRecords
  DEF ActivationInductiveInvariant, ProcessInvariant, SealOutcomes, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealArchive
  DEF ActivationInductiveInvariant, ProcessInvariant, SealOutcomes, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealHistory
  DEF ActivationInductiveInvariant, ProcessInvariant, SealOutcomes, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, TypeOK, Phases, NativePhases, PermitPhases,
      IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits,
      Pairs, Triples, Intents, DeclaredIntents, AllSettled, Settled, SealOutcomes, home, records, archives,
      process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM WriteArchivePreservesInvariant ==
  ASSUME ActivationInductiveInvariant, WriteArchive
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. WriteArchive
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, WriteArchive
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, WriteArchive
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, WriteArchive, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, WriteArchive, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, ArchiveInvariant, ProcessInvariant, WriteArchive
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, WriteArchive, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, WriteArchive, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM ArchiveBarrierPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, ArchiveBarrier
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. ArchiveBarrier
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, ArchiveBarrier
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, ArchiveBarrier
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, ArchiveBarrier
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, ArchiveBarrier, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, ArchiveInvariant, ProcessInvariant, ArchiveBarrier
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, ArchiveBarrier, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, ArchiveBarrier, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM WritebackCurrentPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, WritebackCurrent
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. WritebackCurrent
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, WritebackCurrent
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, WritebackCurrent
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, WritebackCurrent
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, WritebackCurrent, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, WritebackCurrent, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, WritebackCurrent, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, WritebackCurrent, home, records, archives, process,
      history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM WritebackPendingPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, WritebackPending
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. WritebackPending
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2
  DEF ActivationInductiveInvariant, TypeOK, WritebackPending, home, records, archives, process, history
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2
  DEF ActivationInductiveInvariant, PlanInvariant, PointerHasPlan, WritebackPending,
      home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, WritebackPending
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, WritebackPending, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, WritebackPending, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, WritebackPending, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, WritebackPending, home, records, archives, process,
      history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM WritebackOutcomesPreservesInvariant ==
  ASSUME NEW transaction \in TXS, ActivationInductiveInvariant, WritebackOutcomes(transaction)
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant /\ transaction \in TXS
  OBVIOUS
<1>2. WritebackOutcomes(transaction)
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, WritebackOutcomes
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealPlan
  DEF ActivationInductiveInvariant, TypeOK, WritebackOutcomes, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, WritebackOutcomes, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeBarrierRecords
  DEF ActivationInductiveInvariant, ProcessInvariant, WritebackOutcomes, home, records, archives, process,
      history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealArchive
  DEF ActivationInductiveInvariant, ProcessInvariant, WritebackOutcomes, home, records, archives, process,
      history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, OutcomeSealHistory
  DEF ActivationInductiveInvariant, ProcessInvariant, WritebackOutcomes, home, records, archives, process,
      history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, TypeOK, RecordInvariant, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple,
      Permits, Pairs, Triples, Intents, DeclaredIntents, AllSettled, Settled, WritebackOutcomes, home, records,
      archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM WritebackArchivePreservesInvariant ==
  ASSUME NEW transaction \in TXS, ActivationInductiveInvariant, WritebackArchive(transaction)
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant /\ transaction \in TXS
  OBVIOUS
<1>2. WritebackArchive(transaction)
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, WritebackArchive
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      PlanInvariant, PointerHasPlan, ProcessInvariant, WritebackArchive
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, WritebackArchive
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, WritebackArchive, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      RecordInvariant, ArchiveInvariant, ProcessInvariant, WritebackArchive
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, WritebackArchive, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, WritebackArchive, home, records, archives, process,
      history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM ProcessDeathPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, ProcessDeath
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. ProcessDeath
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, ProcessDeath
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2, PlanPredicateFrame
  DEF ActivationInductiveInvariant, ProcessDeath, home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, HomePredicateFrame
  DEF ActivationInductiveInvariant, ProcessDeath, home, records, archives, process, history
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, RecordPredicateFrame
  DEF ActivationInductiveInvariant, ProcessDeath, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2, ArchivePredicateFrame
  DEF ActivationInductiveInvariant, ProcessDeath, home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, HistoryPredicateFrame
  DEF ActivationInductiveInvariant, ProcessDeath, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, ProcessDeath, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

THEOREM PowerLossPreservesInvariant ==
  ASSUME ActivationInductiveInvariant, PowerLoss
  PROVE ActivationInductiveInvariant'
<1>1. ActivationInductiveInvariant
  OBVIOUS
<1>2. PowerLoss
  OBVIOUS
<1>3. TypeOK'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      ProcessInvariant, PowerLoss
<1>4. PlanInvariant'
  BY ONLY SMT, <1>1, <1>2
  DEF ActivationInductiveInvariant, TypeOK, PlanInvariant, PointerHasPlan, PowerLoss,
      home, records, archives, process, history
<1>5. HomeInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, home, records, archives, process, history, Phases, NativePhases,
      PermitPhases, IndexedPhases, CoherentPhases, LoadedPhases, ArchivePhases, State, Pending, Terminal, Pair,
      Triple, Intents, DeclaredIntents, Attempts, Pairs, Triples, Permits, AllSettled, Settled, TypeOK,
      HomeInvariant, NoSilentSkip, ProcessInvariant, PowerLoss
<1>6. RecordInvariant'
  BY ONLY SMT, <1>1, <1>2, PowerLossRecords
  DEF ActivationInductiveInvariant, PowerLoss, home, records, archives, process, history
<1>7. ArchiveInvariant'
  BY ONLY SMT, <1>1, <1>2
  DEF ActivationInductiveInvariant, TypeOK, ArchiveInvariant, PowerLoss,
      home, records, archives, process, history
<1>8. HistoryInvariant'
  BY ONLY SMT, <1>1, <1>2, PowerLossHistory
  DEF ActivationInductiveInvariant, PowerLoss, home, records, archives, process, history
<1>9. ProcessInvariant'
  BY ONLY SMT, <1>1, <1>2, ActivationParameters, CorrectProtocol, DeclaredIntentDomain
  DEF ActivationInductiveInvariant, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases, State, Terminal, Pair, Triple, Permits, Pairs, Triples,
      Intents, DeclaredIntents, AllSettled, Settled, PowerLoss, home, records, archives, process, history
<1>10. QED
  BY <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9 DEF ActivationInductiveInvariant

=============================================================================
