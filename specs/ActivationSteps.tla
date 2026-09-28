---- MODULE ActivationSteps ----
EXTENDS ActivationRecordProofs
ASSUME CorrectProtocol == MUTANT = "none"

THEOREM DeclaredIntentDomain ==
  ASSUME NEW transaction \in TXS
  PROVE DeclaredIntents(transaction) \subseteq Intents
  BY SMT, ActivationParameters DEF DeclaredIntents, Intents

THEOREM EarlySuccessIsDisabled == ~EarlySuccess
  BY ONLY SMT, CorrectProtocol DEF EarlySuccess

THEOREM SealHomePreservesInvariant ==
  ASSUME ActivationInductiveInvariant, SealHome
  PROVE ActivationInductiveInvariant'
<1>1. TypeOK'
  BY SMT, ActivationParameters
  DEF ActivationInductiveInvariant, TypeOK, Phases, SealHome, home, records, archives, process, history
<1>2. PlanInvariant'
  BY SMT
  DEF ActivationInductiveInvariant, PlanInvariant, PointerHasPlan, SealHome,
      home, records, archives, process, history
<1>3. HomeInvariant'
  BY SMT
  DEF ActivationInductiveInvariant, HomeInvariant, NoSilentSkip, SealHome,
      home, records, archives, process, history
<1>4. RecordInvariant' /\ ArchiveInvariant' /\ HistoryInvariant'
  BY SMT
  DEF ActivationInductiveInvariant, RecordInvariant, ArchiveInvariant, HistoryInvariant,
      OutcomeAfterReturn, TerminalNoReplay, EffectsBindFullSelection, InvocationUsesDeclaredIntent,
      SealHome, home, records, archives, process, history
<1>5. ProcessInvariant'
  BY SMT
  DEF ActivationInductiveInvariant, ProcessInvariant, SealHome, CoherentPhases, NativePhases,
      PermitPhases, IndexedPhases, LoadedPhases, ArchivePhases, Phases,
      home, records, archives, process, history
<1>6. QED
  BY <1>1, <1>2, <1>3, <1>4, <1>5 DEF ActivationInductiveInvariant

=============================================================================
