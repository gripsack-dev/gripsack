---- MODULE ActivationBridgeProofs ----
EXTENDS ActivationLifecycle, PublicationSelectionProofs

ASSUME CorrectActivation == ActivationMutant = "none"
HookProofs == INSTANCE ActivationProofs
    WITH TXS <- Transactions, GENS <- GenerationIds, NONE <- ActivationNone,
         MUTANT <- ActivationMutant, phase <- activationPhase

THEOREM HookStateNamesAgree ==
    /\ Hooks!Init = HookProofs!Init
    /\ Hooks!ActivationInductiveInvariant = HookProofs!ActivationInductiveInvariant
    /\ Hooks!ActivationInductiveInvariant' = HookProofs!ActivationInductiveInvariant'
  BY DEF Hooks!Init, HookProofs!Init,
      Hooks!ActivationInductiveInvariant, HookProofs!ActivationInductiveInvariant,
      Hooks!TypeOK, HookProofs!TypeOK,
      Hooks!PlanInvariant, HookProofs!PlanInvariant,
      Hooks!HomeInvariant, HookProofs!HomeInvariant,
      Hooks!RecordInvariant, HookProofs!RecordInvariant,
      Hooks!ArchiveInvariant, HookProofs!ArchiveInvariant,
      Hooks!HistoryInvariant, HookProofs!HistoryInvariant,
      Hooks!ProcessInvariant, HookProofs!ProcessInvariant,
      Hooks!PointerHasPlan, HookProofs!PointerHasPlan,
      Hooks!NoSilentSkip, HookProofs!NoSilentSkip,
      Hooks!OutcomeAfterReturn, HookProofs!OutcomeAfterReturn,
      Hooks!TerminalNoReplay, HookProofs!TerminalNoReplay,
      Hooks!EffectsBindFullSelection, HookProofs!EffectsBindFullSelection,
      Hooks!InvocationUsesDeclaredIntent, HookProofs!InvocationUsesDeclaredIntent,
      Hooks!Intents, HookProofs!Intents,
      Hooks!DeclaredIntents, HookProofs!DeclaredIntents,
      Hooks!Attempts, HookProofs!Attempts,
      Hooks!Pending, HookProofs!Pending,
      Hooks!State, HookProofs!State,
      Hooks!States, HookProofs!States,
      Hooks!Permits, HookProofs!Permits,
      Hooks!Phases, HookProofs!Phases,
      Hooks!Pairs, HookProofs!Pairs,
      Hooks!Triples, HookProofs!Triples,
      Hooks!Pair, HookProofs!Pair,
      Hooks!Triple, HookProofs!Triple,
      Hooks!Terminal, HookProofs!Terminal,
      Hooks!AllSettled, HookProofs!AllSettled,
      Hooks!NativePhases, HookProofs!NativePhases,
      Hooks!PermitPhases, HookProofs!PermitPhases,
      Hooks!IndexedPhases, HookProofs!IndexedPhases,
      Hooks!CoherentPhases, HookProofs!CoherentPhases,
      Hooks!LoadedPhases, HookProofs!LoadedPhases,
      Hooks!ArchivePhases, HookProofs!ArchivePhases

THEOREM HookVariableFramesAgree ==
    /\ ((UNCHANGED Hooks!vars) <=> (UNCHANGED hookVars))
    /\ ((UNCHANGED HookProofs!vars) <=> (UNCHANGED hookVars))
  BY SMT DEF Hooks!vars, HookProofs!vars, Hooks!home, HookProofs!home,
      Hooks!records, HookProofs!records, Hooks!archives, HookProofs!archives,
      Hooks!process, HookProofs!process, Hooks!history, HookProofs!history, hookVars

THEOREM HookFiniteDomainsAgree ==
    \A values : Hooks!IsFiniteSet(values) = HookProofs!IsFiniteSet(values)
  BY DEF Hooks!IsFiniteSet, HookProofs!IsFiniteSet

THEOREM HookProofPremises ==
    HookProofs!ActivationParameters /\ HookProofs!OptionalValueDomain /\ HookProofs!CorrectProtocol
  BY SMT, HookDomain, CorrectActivation, HookFiniteDomainsAgree
  DEF Hooks!ActivationParameters, HookProofs!ActivationParameters,
      Hooks!OptionalValueDomain, HookProofs!OptionalValueDomain, HookProofs!CorrectProtocol,
      Hooks!Permits, HookProofs!Permits, Hooks!Intents, HookProofs!Intents,
      Hooks!Attempts, HookProofs!Attempts

THEOREM HookTransitionNamesAgree == Hooks!Next = HookProofs!Next
  BY DEF Hooks!Next, HookProofs!Next,
      Hooks!Progress, HookProofs!Progress,
      Hooks!Prepare, HookProofs!Prepare,
      Hooks!WritePointer, HookProofs!WritePointer,
      Hooks!SyncPointer, HookProofs!SyncPointer,
      Hooks!Flip, HookProofs!Flip,
      Hooks!SyncFlip, HookProofs!SyncFlip,
      Hooks!Open, HookProofs!Open,
      Hooks!SealHome, HookProofs!SealHome,
      Hooks!SealOutcomes, HookProofs!SealOutcomes,
      Hooks!Authorize, HookProofs!Authorize,
      Hooks!Skip, HookProofs!Skip,
      Hooks!Start, HookProofs!Start,
      Hooks!StartBarrier, HookProofs!StartBarrier,
      Hooks!Invoke, HookProofs!Invoke,
      Hooks!Return, HookProofs!Return,
      Hooks!WriteOutcome, HookProofs!WriteOutcome,
      Hooks!OutcomeBarrier, HookProofs!OutcomeBarrier,
      Hooks!SkipSuperseded, HookProofs!SkipSuperseded,
      Hooks!Supersede, HookProofs!Supersede,
      Hooks!SupersedeBarrier, HookProofs!SupersedeBarrier,
      Hooks!FinishSuperseding, HookProofs!FinishSuperseding,
      Hooks!FinishScan, HookProofs!FinishScan,
      Hooks!WriteArchive, HookProofs!WriteArchive,
      Hooks!ArchiveBarrier, HookProofs!ArchiveBarrier,
      Hooks!Clear, HookProofs!Clear,
      Hooks!ClearBarrier, HookProofs!ClearBarrier,
      Hooks!EarlySuccess, HookProofs!EarlySuccess,
      Hooks!SelectWithoutActivation, HookProofs!SelectWithoutActivation,
      Hooks!SealWithoutActivation, HookProofs!SealWithoutActivation,
      Hooks!WritebackCurrent, HookProofs!WritebackCurrent,
      Hooks!WritebackPending, HookProofs!WritebackPending,
      Hooks!WritebackOutcomes, HookProofs!WritebackOutcomes,
      Hooks!WritebackArchive, HookProofs!WritebackArchive,
      Hooks!ProcessDeath, HookProofs!ProcessDeath,
      Hooks!PowerLoss, HookProofs!PowerLoss,
      Hooks!CanStart, HookProofs!CanStart,
      Hooks!Matches, HookProofs!Matches,
      Hooks!Settled, HookProofs!Settled,
      Hooks!home, HookProofs!home,
      Hooks!records, HookProofs!records,
      Hooks!archives, HookProofs!archives,
      Hooks!process, HookProofs!process,
      Hooks!history, HookProofs!history,
      Hooks!State, HookProofs!State,
      Hooks!Terminal, HookProofs!Terminal,
      Hooks!AllSettled, HookProofs!AllSettled,
      Hooks!DeclaredIntents, HookProofs!DeclaredIntents,
      Hooks!Pair, HookProofs!Pair,
      Hooks!Triple, HookProofs!Triple,
      Hooks!Intents, HookProofs!Intents

THEOREM HookComponentInitialSafety == Hooks!Init => Hooks!ActivationInductiveInvariant
  BY SMT, HookStateNamesAgree, HookProofPremises, HookProofs!ActivationInitialSafety

THEOREM HookComponentInduction ==
    Hooks!ActivationInductiveInvariant /\ Hooks!Next => Hooks!ActivationInductiveInvariant'
  BY SMT, HookStateNamesAgree, HookTransitionNamesAgree, HookProofPremises, HookProofs!ActivationInduction

THEOREM HookStutterPreservesInvariant ==
  ASSUME Hooks!ActivationInductiveInvariant, UNCHANGED hookVars
  PROVE Hooks!ActivationInductiveInvariant'
  BY SMT, HookStateNamesAgree, HookVariableFramesAgree, HookProofPremises, HookProofs!ActivationPredicateFrame

THEOREM LifecycleProjectsHookTransitions ==
  ASSUME NEW admitted \in BOOLEAN, ActivationLifecycleNext(admitted)
  PROVE [Hooks!Next]_hookVars
  BY SMT DEF ActivationLifecycleNext, PublicationWithHooks, ObservedGenerationWithHooks,
      BeginLifecycleEpoch, StationaryJournalStep, PrepareActivationPlan, WriteActivationPointer,
      SealActivationPointer, FlipLifecycleSelection, SealLifecycleCommit, HookCurrentWriteback,
      WritebackLifecycleCurrent, ActivationExecutionStep, ActivationHomeBarrier, ActivationStorageStep,
      LifecycleProcessCrash, LifecycleStorageCrash, Hooks!Next, Hooks!Progress

THEOREM LifecycleProjectsPublicationSelection ==
  ASSUME NEW admitted \in BOOLEAN, ActivationLifecycleNext(admitted)
  PROVE [LifecycleNext(admitted)]_lifecycleVars
  BY SMT DEF ActivationLifecycleNext, PublicationWithHooks, ObservedGenerationWithHooks,
      BeginLifecycleEpoch, StationaryJournalStep, PrepareActivationPlan, WriteActivationPointer,
      SealActivationPointer, FlipLifecycleSelection, SealLifecycleCommit, WritebackLifecycleCurrent,
      ActivationExecutionStep, ActivationHomeBarrier, ActivationStorageStep,
      LifecycleProcessCrash, LifecycleStorageCrash, JournalHomeBarrier,
      LifecycleNext, JournalStep, CoreNext, WritebackSelection, lifecycleVars

=============================================================================
