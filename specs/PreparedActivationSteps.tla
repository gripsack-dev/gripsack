---- MODULE PreparedActivationSteps ----
EXTENDS PreparedActivationBridgeProofs

PreparationHistoryInvariant == PreparationTypes /\ PreparationOwnerBinding /\ EveryPlanHasDurablePreparation

THEOREM ActivationLifecycleStateFrame ==
  ASSUME ActivationLifecycleInvariant, UNCHANGED activationLifecycleVars
  PROVE ActivationLifecycleInvariant'
  BY SMT, PublicationSelectionInvariantFrame, HookStutterPreservesInvariant, CouplingStateFrame
  DEF ActivationLifecycleInvariant, CouplingInvariant, activationLifecycleVars, couplingVars,
      lifecycleVars, vars, selectionVars, control, hookVars

THEOREM DurableSnapshotIsTyped == Preparation!Invariant => DurablePreparationSnapshot \in SnapshotSpace
  BY SMT DEF Preparation!Invariant, Preparation!TypeOK, SnapshotSpace, DurablePreparationSnapshot

THEOREM SnapshotCoversDeclaredDocuments ==
  ASSUME NEW transaction \in Transactions, PreparedLifecycleInvariant,
         preparationOwner = transaction, preparationReturned
  PROVE /\ RequiredPreparationDocuments(transaction) \subseteq DurablePreparationSnapshot.files
              \intersect DurablePreparationSnapshot.modes \intersect DurablePreparationSnapshot.names
        /\ DurablePreparationSnapshot.namespace
  BY SMT, HookDomain
  DEF PreparedLifecycleInvariant, PreparationOwnerBinding, Preparation!Invariant,
      Preparation!PreparedDocumentsAreDurable, DurablePreparationSnapshot, Hooks!ActivationParameters

THEOREM PreparationIOKeepsInputAndCoordinator ==
  PreparationIO => UNCHANGED <<preparationRequested, preparationStopped>>
  BY SMT DEF PreparationIO, Preparation!Next, Preparation!PublicationStep,
      Preparation!WriteBytes, Preparation!SetPrivateMode, Preparation!SyncFile,
      Preparation!PublishName, Preparation!SyncDocumentParent, Preparation!SealNamespace,
      Preparation!ReturnPreparation, Preparation!WritebackDocuments, Preparation!WritebackNamespace,
      Preparation!publicationVars

THEOREM OrdinaryLifecycleKeepsPlanBytes ==
  ASSUME NEW admitted, OrdinaryPreparedLifecycleStep(admitted)
  PROVE UNCHANGED plans
<1>1. CASE PublicationWithHooks \/ (\E generation \in GenerationIds : ObservedGenerationWithHooks(generation))
           \/ (\E transaction \in Transactions, generation \in GenerationIds : BeginLifecycleEpoch(transaction, generation))
           \/ StationaryJournalStep(admitted)
  BY SMT, <1>1 DEF PublicationWithHooks, ObservedGenerationWithHooks, BeginLifecycleEpoch,
      StationaryJournalStep, hookVars
<1>2. CASE WriteActivationPointer \/ SealActivationPointer \/ FlipLifecycleSelection \/
           SealLifecycleCommit \/ WritebackLifecycleCurrent \/ ActivationHomeBarrier
  BY SMT, <1>2
  DEF WriteActivationPointer, SealActivationPointer, FlipLifecycleSelection, SealLifecycleCommit,
      WritebackLifecycleCurrent, ActivationHomeBarrier, HookCurrentWriteback,
      Hooks!WritePointer, Hooks!SyncPointer, Hooks!Flip, Hooks!SelectWithoutActivation,
      Hooks!SyncFlip, Hooks!WritebackCurrent, Hooks!SealHome, Hooks!ClearBarrier, Hooks!SealWithoutActivation,
      hookVars
<1>3. CASE ActivationExecutionStep \/ ActivationStorageStep
  BY SMT, <1>3
  DEF ActivationExecutionStep, ActivationStorageStep, Hooks!Open, Hooks!SealOutcomes, Hooks!Authorize,
      Hooks!Skip, Hooks!Start, Hooks!StartBarrier, Hooks!Invoke, Hooks!Return, Hooks!WriteOutcome,
      Hooks!OutcomeBarrier, Hooks!Supersede, Hooks!SkipSuperseded, Hooks!SupersedeBarrier,
      Hooks!FinishSuperseding, Hooks!FinishScan, Hooks!WriteArchive, Hooks!ArchiveBarrier,
      Hooks!Clear, Hooks!EarlySuccess, Hooks!WritebackPending, Hooks!WritebackOutcomes, Hooks!WritebackArchive
<1>4. QED BY ONLY SMT, OrdinaryPreparedLifecycleStep(admitted), <1>1, <1>2, <1>3
  DEF OrdinaryPreparedLifecycleStep

THEOREM OrdinaryLifecycleKeepsActiveConstructor ==
  ASSUME NEW admitted, PreparationTypes, PreparationOwnerBinding,
         preparationOwner # ActivationNone, OrdinaryPreparedLifecycleStep(admitted)
  PROVE UNCHANGED <<mode, pending, activationPhase, pendingC, plans>>
<1>1. mode = "publishing" /\ activationPhase = "idle" /\ pendingC = ActivationNone /\ NeedsActivation(pending)
  BY SMT DEF PreparationTypes, PreparationOwnerBinding, NeedsActivation
<1>2. UNCHANGED plans
  BY OrdinaryLifecycleKeepsPlanBytes
<1>3. QED
  BY SMT, <1>1, <1>2, CorrectJournal
  DEF OrdinaryPreparedLifecycleStep, PublicationWithHooks, PublicationStep, ObservedGenerationWithHooks,
      AdmitObservedGeneration, BeginLifecycleEpoch, BeginEpoch, StationaryJournalStep, PrepareSelection,
      WriteRunMarker, SealRunMarker, WriterStep, RestoreStep, RemoveEntry, FinishWriting, FinishNoop,
      ClassifyRecovery, RetryRecovery, FinishRestoring, SealEntryRemoval, RemoveMarker, AlreadyMissingMarker,
      SealMarkerRemoval, WritebackDestination, ExternalDestination, Selection!SealReservations, Selection!SealMarker,
      WriteActivationPointer, SealActivationPointer, FlipLifecycleSelection, SealLifecycleCommit,
      FlipSelection, SealCommittedSelection, WritebackLifecycleCurrent, HookCurrentWriteback,
      ActivationExecutionStep, ActivationHomeBarrier, ActivationStorageStep, JournalHomeBarrier,
      Selection!SealCurrent, Hooks!WritePointer, Hooks!SyncPointer, Hooks!Flip, Hooks!WritebackCurrent,
      Hooks!WritebackPending, Hooks!WritebackOutcomes, Hooks!WritebackArchive,
      activationLifecycleVars, lifecycleVars, vars, selectionVars, control, hookVars, Hooks!home, Hooks!process

THEOREM BeginPreparationPreservesHistory ==
  ASSUME NEW transaction \in Transactions, PreparedLifecycleInvariant, BeginPreparation(transaction)
  PROVE PreparationHistoryInvariant'
  BY SMT
  DEF PreparedLifecycleInvariant, PreparationHistoryInvariant, PreparationTypes, PreparationOwnerBinding,
      EveryPlanHasDurablePreparation, BeginPreparation, Preparation!ResetFor,
      activationLifecycleVars, lifecycleVars, vars, selectionVars, control, hookVars

THEOREM PreparationIOPreservesHistory ==
  ASSUME PreparedLifecycleInvariant, PreparationIO
  PROVE PreparationHistoryInvariant'
  BY SMT, PreparationIOKeepsInputAndCoordinator
  DEF PreparedLifecycleInvariant, PreparationHistoryInvariant, PreparationTypes, PreparationOwnerBinding,
      EveryPlanHasDurablePreparation, PreparationIO, preparationHistoryVars,
      activationLifecycleVars, lifecycleVars, vars, selectionVars, control, hookVars

THEOREM FinishPreparationPreservesHistory ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds,
         PreparedLifecycleInvariant, FinishPreparation(transaction, generation)
  PROVE PreparationHistoryInvariant'
<1>1. DurablePreparationSnapshot \in SnapshotSpace
  BY SMT, DurableSnapshotIsTyped DEF PreparedLifecycleInvariant
<1>2. RequiredPreparationDocuments(transaction) \subseteq DurablePreparationSnapshot.files
        \intersect DurablePreparationSnapshot.modes \intersect DurablePreparationSnapshot.names /\
      DurablePreparationSnapshot.namespace
  BY SMT, SnapshotCoversDeclaredDocuments DEF FinishPreparation
<1>3. transaction \in preparationStarted
  BY SMT, HookDomain
  DEF PreparedLifecycleInvariant, PreparationOwnerBinding, FinishPreparation, Hooks!ActivationParameters
<1>4. QED
  BY SMT, <1>1, <1>2, <1>3
  DEF PreparedLifecycleInvariant, PreparationHistoryInvariant, PreparationTypes, PreparationOwnerBinding,
      EveryPlanHasDurablePreparation, FinishPreparation, PrepareActivationPlan, Hooks!Prepare,
      preparationVars, ActivationLifecycleInvariant, Hooks!ActivationInductiveInvariant, Hooks!TypeOK

THEOREM OrdinaryPreparedStepPreservesHistory ==
  ASSUME NEW admitted, PreparedLifecycleInvariant, OrdinaryPreparedLifecycleStep(admitted)
  PROVE PreparationHistoryInvariant'
  BY SMT, OrdinaryLifecycleKeepsPlanBytes, OrdinaryLifecycleKeepsActiveConstructor
  DEF PreparedLifecycleInvariant, PreparationHistoryInvariant, PreparationTypes, PreparationOwnerBinding,
      EveryPlanHasDurablePreparation, OrdinaryPreparedLifecycleStep, preparationVars, preparationHistoryVars

THEOREM PreparedProcessCrashPreservesHistory ==
  ASSUME PreparedLifecycleInvariant, PreparedProcessCrash
  PROVE PreparationHistoryInvariant'
  BY SMT
  DEF PreparedLifecycleInvariant, PreparationHistoryInvariant, PreparationTypes, PreparationOwnerBinding,
      EveryPlanHasDurablePreparation, PreparedProcessCrash, LifecycleProcessCrash, Hooks!ProcessDeath,
      hookVars

THEOREM PreparedStorageCrashPreservesHistory ==
  ASSUME PreparedLifecycleInvariant, PreparedStorageCrash
  PROVE PreparationHistoryInvariant'
  BY SMT
  DEF PreparedLifecycleInvariant, PreparationHistoryInvariant, PreparationTypes, PreparationOwnerBinding,
      EveryPlanHasDurablePreparation, PreparedStorageCrash, LifecycleStorageCrash, Hooks!PowerLoss,
      hookVars

=============================================================================
