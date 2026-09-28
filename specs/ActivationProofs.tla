---- MODULE ActivationProofs ----
EXTENDS ActivationPointerProofs, ActivationIntentProofs, ActivationStorageProofs

THEOREM PendingStateType == Pending \in States
  BY SMT DEF States

THEOREM ActivationInitialSafety == Init => ActivationInductiveInvariant
<1> SUFFICES ASSUME Init PROVE ActivationInductiveInvariant
  OBVIOUS
<1>1. TypeOK
  BY SMT, ActivationParameters, PendingStateType
  DEF Init, TypeOK, Phases, Permits, Intents, Attempts, Pairs, Triples
<1>2. PlanInvariant
  BY SMT DEF Init, PlanInvariant, PointerHasPlan
<1>3. HomeInvariant
  BY SMT DEF Init, HomeInvariant, NoSilentSkip
<1>4. RecordInvariant
  BY SMT DEF Init, RecordInvariant, Terminal, Pending, State
<1>5. ArchiveInvariant
  BY SMT DEF Init, ArchiveInvariant
<1>6. HistoryInvariant
  BY SMT DEF Init, HistoryInvariant, OutcomeAfterReturn, TerminalNoReplay,
      EffectsBindFullSelection, InvocationUsesDeclaredIntent, Pending, State
<1>7. ProcessInvariant
  BY SMT
  DEF Init, ProcessInvariant, Phases, NativePhases, PermitPhases, IndexedPhases,
      CoherentPhases, LoadedPhases, ArchivePhases
<1>8. QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF ActivationInductiveInvariant

THEOREM ActivationInduction == ActivationInductiveInvariant /\ Next => ActivationInductiveInvariant'
  BY SMT, SealHomePreservesInvariant, EarlySuccessIsDisabled,
     PreparePreservesInvariant,
     WritePointerPreservesInvariant,
     SyncPointerPreservesInvariant,
     FlipPreservesInvariant,
     SyncFlipPreservesInvariant,
     OpenPreservesInvariant,
     AuthorizePreservesInvariant,
     ClearPreservesInvariant,
     ClearBarrierPreservesInvariant,
     SelectWithoutActivationPreservesInvariant,
     SealWithoutActivationPreservesInvariant,
     SkipPreservesInvariant,
     StartPreservesInvariant,
     StartBarrierPreservesInvariant,
     InvokePreservesInvariant,
     ReturnPreservesInvariant,
     WriteOutcomePreservesInvariant,
     OutcomeBarrierPreservesInvariant,
     SkipSupersededPreservesInvariant,
     SupersedePreservesInvariant,
     SupersedeBarrierPreservesInvariant,
     FinishSupersedingPreservesInvariant,
     FinishScanPreservesInvariant,
     SealOutcomesPreservesInvariant,
     WriteArchivePreservesInvariant,
     ArchiveBarrierPreservesInvariant,
     WritebackCurrentPreservesInvariant,
     WritebackPendingPreservesInvariant,
     WritebackOutcomesPreservesInvariant,
     WritebackArchivePreservesInvariant,
     ProcessDeathPreservesInvariant,
     PowerLossPreservesInvariant
  DEF Next, Progress

THEOREM ActivationInvariantImpliesSafety == ActivationInductiveInvariant =>
    /\ PointerHasPlan /\ PermitAfterDurableStart /\ OutcomeAfterReturn
    /\ TerminalNoReplay /\ EffectsBindFullSelection /\ ArchiveBeforeClear
    /\ ArchiveHasTerminalOutcomes /\ NoSilentSkip /\ DistinctIntentIdentity
    /\ InvocationUsesDeclaredIntent /\ GcSafePendingResurrection /\ IndependentHomeResolution
<1> SUFFICES ASSUME ActivationInductiveInvariant
               PROVE /\ PointerHasPlan /\ PermitAfterDurableStart /\ OutcomeAfterReturn
                     /\ TerminalNoReplay /\ EffectsBindFullSelection /\ ArchiveBeforeClear
                     /\ ArchiveHasTerminalOutcomes /\ NoSilentSkip /\ DistinctIntentIdentity
                     /\ InvocationUsesDeclaredIntent /\ GcSafePendingResurrection /\ IndependentHomeResolution
  OBVIOUS
<1>1. PointerHasPlan /\ OutcomeAfterReturn /\ TerminalNoReplay /\ EffectsBindFullSelection
       /\ NoSilentSkip /\ InvocationUsesDeclaredIntent
  BY DEF ActivationInductiveInvariant, PlanInvariant, HomeInvariant, HistoryInvariant
<1>2. PermitAfterDurableStart
  BY SMT DEF ActivationInductiveInvariant, ProcessInvariant, PermitAfterDurableStart
<1>3. ArchiveBeforeClear /\ ArchiveHasTerminalOutcomes
  BY SMT
  DEF ActivationInductiveInvariant, TypeOK, ArchiveInvariant, ArchiveBeforeClear, ArchiveHasTerminalOutcomes
<1>4. DistinctIntentIdentity
  BY SMT, CorrectProtocol DEF DistinctIntentIdentity, Token
<1>5. GcSafePendingResurrection
  BY SMT DEF ActivationInductiveInvariant, HomeInvariant, ArchiveInvariant, TypeOK, GcSafePendingResurrection
<1>6. IndependentHomeResolution
  BY SMT DEF ActivationInductiveInvariant, HomeInvariant, IndependentHomeResolution
<1>7. QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6

THEOREM GeneralActivationSafety == Spec => []ActivationInductiveInvariant
<1>1. Init => ActivationInductiveInvariant
  BY ActivationInitialSafety
<1>2. ActivationInductiveInvariant /\ UNCHANGED vars => ActivationInductiveInvariant'
  BY SMT, ActivationPredicateFrame
<1>3. ActivationInductiveInvariant /\ [Next]_vars => ActivationInductiveInvariant'
  BY ActivationInduction, <1>2
<1>4. QED
  BY PTL, <1>1, <1>3 DEF Spec

THEOREM ActivationProtocolSafety == Spec =>
    [](/\ PointerHasPlan /\ PermitAfterDurableStart /\ OutcomeAfterReturn
       /\ TerminalNoReplay /\ EffectsBindFullSelection /\ ArchiveBeforeClear
       /\ ArchiveHasTerminalOutcomes /\ NoSilentSkip /\ DistinctIntentIdentity
       /\ InvocationUsesDeclaredIntent /\ GcSafePendingResurrection /\ IndependentHomeResolution)
  BY PTL, GeneralActivationSafety, ActivationInvariantImpliesSafety

=============================================================================
