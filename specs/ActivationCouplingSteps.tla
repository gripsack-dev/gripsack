---- MODULE ActivationCouplingSteps ----
EXTENDS ActivationCouplingProofs

THEOREM PublicationWithHooksPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, PublicationWithHooks
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF PublicationWithHooks, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, PublicationStep

THEOREM ObservedGenerationWithHooksPreservesCoupling ==
  ASSUME NEW generation \in GenerationIds, ActivationLifecycleInvariant, ObservedGenerationWithHooks(generation)
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF ObservedGenerationWithHooks, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, AdmitObservedGeneration

THEOREM BeginLifecycleEpochPreservesCoupling ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds, ActivationLifecycleInvariant, BeginLifecycleEpoch(transaction, generation)
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation, NewReservationHasNoSavedPlan
  DEF BeginLifecycleEpoch, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, BeginEpoch, Selection!Reserve,
      Invariant, Selection!Invariant, Selection!TypeOK, TypeOK, Hooks!ActivationInductiveInvariant,
      Hooks!TypeOK

THEOREM StationaryJournalStepPreservesCoupling ==
  ASSUME NEW admitted \in BOOLEAN, ActivationLifecycleInvariant, StationaryJournalStep(admitted)
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF StationaryJournalStep, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, PrepareSelection,
      WriteRunMarker, SealRunMarker, WriterStep, RestoreStep, RemoveEntry, FinishWriting, FinishNoop,
      ClassifyRecovery, RetryRecovery, FinishRestoring, SealEntryRemoval, RemoveMarker, AlreadyMissingMarker,
      SealMarkerRemoval, WritebackDestination, ExternalDestination, Selection!SealReservations,
      Selection!SealMarker, Selection!PublishLink, Selection!SealLink, Selection!WriteMarker,
      Selection!ClearMarker, Invariant, Selection!Invariant, Selection!ReservationInvariant

THEOREM PrepareActivationPlanPreservesCoupling ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds, ActivationLifecycleInvariant, PrepareActivationPlan(transaction, generation)
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation, PublishingTargetHasDurableReservation, PublishingTargetIsNotCurrent
  DEF PrepareActivationPlan, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, Hooks!Prepare, Invariant,
      ControlInvariant, Hooks!ActivationInductiveInvariant, Hooks!TypeOK

THEOREM WriteActivationPointerPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, WriteActivationPointer
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF WriteActivationPointer, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, Hooks!WritePointer

THEOREM SealActivationPointerPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, SealActivationPointer
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF SealActivationPointer, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, JournalHomeBarrier,
      Selection!SealCurrent, Hooks!SyncPointer

THEOREM FlipLifecycleSelectionPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, FlipLifecycleSelection
  PROVE CouplingInvariant'
<1>1. PlanSelectionBinding' /\ PreparedPlansHaveWork'
  BY SMT DEF ActivationLifecycleInvariant, PlanSelectionBinding, PreparedPlansHaveWork,
      FlipLifecycleSelection, FlipSelection, Selection!Flip, Hooks!Flip, Hooks!SelectWithoutActivation,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, hookVars
<1>2. PreparationMatchesEpoch'
  BY SMT DEF ActivationLifecycleInvariant, PreparationMatchesEpoch, PreparationPhases,
      FlipLifecycleSelection, FlipSelection, Selection!Flip, Hooks!Flip, Hooks!SelectWithoutActivation,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, hookVars
<1>3. CurrentProjection'
  <2>1. CASE NeedsActivation(pending)
    <3>1. /\ activationPhase = "pre-flip" /\ mode = "publishing" /\ pending # NoSelection
           /\ currentVisible' = pending /\ currentStable' = currentStable
           /\ currentC' = active /\ currentD' = currentD /\ plans' = plans
      BY ONLY SMT, <2>1, FlipLifecycleSelection
      DEF FlipLifecycleSelection, FlipSelection, Selection!Flip, Hooks!Flip
    <3>2. EpochInvariant /\ PreparationMatchesEpoch /\ Hooks!ProcessInvariant
      BY ONLY SMT, ActivationLifecycleInvariant
      DEF ActivationLifecycleInvariant, LifecycleInvariant, Invariant, Hooks!ActivationInductiveInvariant
    <3>3. pending = epochTarget
      BY ONLY SMT, <3>1, <3>2 DEF EpochInvariant
    <3>4. epochTarget = <<active, plans[active]>>
      BY ONLY SMT, <3>1, <3>2 DEF PreparationMatchesEpoch, PreparationPhases
    <3>5. active \in Transactions /\ plans[active] # ActivationNone
      BY ONLY SMT, <3>1, <3>2 DEF Hooks!ProcessInvariant
    <3>6. QED BY ONLY SMT, <3>1, <3>3, <3>4, <3>5, ActivationLifecycleInvariant
      DEF ActivationLifecycleInvariant, CurrentProjection, HookSelection
  <2>2. CASE ~NeedsActivation(pending)
    BY SMT, <2>2
    DEF ActivationLifecycleInvariant, LifecycleInvariant, Invariant, ControlInvariant, EpochInvariant,
        PreparedPlansHaveWork, CurrentProjection, HookSelection, NeedsActivation, Selection!IsTransaction,
        FlipLifecycleSelection, FlipSelection, Selection!Flip, Hooks!SelectWithoutActivation,
        Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, hookVars
  <2>3. QED BY ONLY SMT, <2>1, <2>2
<1>4. QED BY ONLY SMT, <1>1, <1>2, <1>3 DEF CouplingInvariant

THEOREM SealLifecycleCommitPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, SealLifecycleCommit
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF SealLifecycleCommit, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, SealCommittedSelection,
      Selection!SealCurrent, HookCurrentWriteback, Hooks!SyncFlip, Hooks!WritebackCurrent

THEOREM WritebackLifecycleCurrentPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, WritebackLifecycleCurrent
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF WritebackLifecycleCurrent, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, JournalHomeBarrier,
      Selection!SealCurrent, HookCurrentWriteback, Hooks!WritebackCurrent

THEOREM ActivationExecutionStepPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, ActivationExecutionStep
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF ActivationExecutionStep, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, Hooks!Open, Hooks!SealOutcomes,
      Hooks!Authorize, Hooks!Skip, Hooks!Start, Hooks!StartBarrier, Hooks!Invoke, Hooks!Return,
      Hooks!WriteOutcome, Hooks!OutcomeBarrier, Hooks!Supersede, Hooks!SkipSuperseded, Hooks!SupersedeBarrier,
      Hooks!FinishSuperseding, Hooks!FinishScan, Hooks!WriteArchive, Hooks!ArchiveBarrier, Hooks!Clear,
      Hooks!EarlySuccess

THEOREM ActivationHomeBarrierPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, ActivationHomeBarrier
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF ActivationHomeBarrier, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, JournalHomeBarrier,
      Selection!SealCurrent, Hooks!SealHome, Hooks!ClearBarrier, Hooks!SyncFlip, Hooks!SealWithoutActivation

THEOREM ActivationStorageStepPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, ActivationStorageStep
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF ActivationStorageStep, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, Hooks!WritebackPending,
      Hooks!WritebackOutcomes, Hooks!WritebackArchive

THEOREM LifecycleProcessCrashPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, LifecycleProcessCrash
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF LifecycleProcessCrash, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, LifecycleProcessDeath,
      CrashProcess, Selection!ProcessDeath, Hooks!ProcessDeath

THEOREM LifecycleStorageCrashPreservesCoupling ==
  ASSUME ActivationLifecycleInvariant, LifecycleStorageCrash
  PROVE CouplingInvariant'
  BY SMT, CorrectJournal, CorrectActivation
  DEF LifecycleStorageCrash, ActivationLifecycleInvariant, LifecycleInvariant, CouplingInvariant,
      PlanSelectionBinding, CurrentProjection, PreparationMatchesEpoch, PreparedPlansHaveWork,
      PreparationPhases, HookSelection, lifecycleVars, vars, selectionVars, control, publicationVars, hookVars,
      Hooks!home, Hooks!records, Hooks!archives, Hooks!process, Hooks!history, LifecyclePowerLoss, CrashPower,
      Selection!PowerLoss, Hooks!PowerLoss

=============================================================================
