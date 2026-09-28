---- MODULE ActivationLifecycleProofs ----
EXTENDS ActivationCouplingSteps

THEOREM PublicationSelectionInvariantFrame ==
  ASSUME LifecycleInvariant, UNCHANGED lifecycleVars
  PROVE LifecycleInvariant'
  BY SMT, JournalStutterPreservesInvariant, PublicationStutterPreservesInvariant
  DEF LifecycleInvariant, CurrentNamesDurableGeneration, lifecycleVars, vars, selectionVars, publicationVars

THEOREM ActivationLifecycleInitialSafety == ActivationLifecycleInit => ActivationLifecycleInvariant
<1>1. ActivationLifecycleInit => LifecycleInvariant /\ Hooks!ActivationInductiveInvariant
  BY SMT, LifecycleInitialSafety, HookComponentInitialSafety DEF ActivationLifecycleInit
<1>2. ActivationLifecycleInit => CouplingInvariant
  BY SMT, SelectionDomain
  DEF ActivationLifecycleInit, LifecycleInit, Init, Selection!Init, Selection!Parameters,
      Hooks!Init, CouplingInvariant, PlanSelectionBinding, CurrentProjection, HookSelection,
      PreparationMatchesEpoch, PreparationPhases, PreparedPlansHaveWork
<1>3. QED BY <1>1, <1>2 DEF ActivationLifecycleInvariant, CouplingInvariant

THEOREM ActivationLifecycleComponentInduction ==
  ASSUME NEW admitted \in BOOLEAN, ActivationLifecycleInvariant, ActivationLifecycleNext(admitted)
  PROVE LifecycleInvariant' /\ Hooks!ActivationInductiveInvariant'
<1>1. [LifecycleNext(admitted)]_lifecycleVars
  BY LifecycleProjectsPublicationSelection
<1>2. LifecycleInvariant'
  BY SMT, <1>1, LifecycleInduction, PublicationSelectionInvariantFrame DEF ActivationLifecycleInvariant
<1>3. [Hooks!Next]_hookVars
  BY LifecycleProjectsHookTransitions
<1>4. Hooks!ActivationInductiveInvariant'
  BY SMT, <1>3, HookComponentInduction, HookStutterPreservesInvariant DEF ActivationLifecycleInvariant
<1>5. QED BY <1>2, <1>4

THEOREM ActivationLifecycleCouplingInduction ==
  ASSUME NEW admitted \in BOOLEAN, ActivationLifecycleInvariant, ActivationLifecycleNext(admitted)
  PROVE CouplingInvariant'
  BY SMT, PublicationWithHooksPreservesCoupling, ObservedGenerationWithHooksPreservesCoupling,
      BeginLifecycleEpochPreservesCoupling, StationaryJournalStepPreservesCoupling,
      PrepareActivationPlanPreservesCoupling, WriteActivationPointerPreservesCoupling,
      SealActivationPointerPreservesCoupling, FlipLifecycleSelectionPreservesCoupling,
      SealLifecycleCommitPreservesCoupling, WritebackLifecycleCurrentPreservesCoupling,
      ActivationExecutionStepPreservesCoupling, ActivationHomeBarrierPreservesCoupling,
      ActivationStorageStepPreservesCoupling, LifecycleProcessCrashPreservesCoupling,
      LifecycleStorageCrashPreservesCoupling
  DEF ActivationLifecycleNext

THEOREM ActivationLifecycleInduction ==
  ASSUME NEW admitted \in BOOLEAN, ActivationLifecycleInvariant, ActivationLifecycleNext(admitted)
  PROVE ActivationLifecycleInvariant'
  BY ActivationLifecycleComponentInduction, ActivationLifecycleCouplingInduction
  DEF ActivationLifecycleInvariant, CouplingInvariant

THEOREM GeneralActivationLifecycleSafety == ActivationLifecycleSpec => []ActivationLifecycleInvariant
<1>1. ActivationLifecycleInit => ActivationLifecycleInvariant
  BY ActivationLifecycleInitialSafety
<1>2. ActivationLifecycleInvariant /\ UNCHANGED activationLifecycleVars => ActivationLifecycleInvariant'
  BY SMT, PublicationSelectionInvariantFrame, HookStutterPreservesInvariant, CouplingStateFrame
  DEF ActivationLifecycleInvariant, CouplingInvariant, activationLifecycleVars, couplingVars,
      lifecycleVars, vars, selectionVars, control, hookVars
<1>3. ActivationLifecycleInvariant /\ [ActivationLifecycleNext(TRUE)]_activationLifecycleVars => ActivationLifecycleInvariant'
  BY ActivationLifecycleInduction, <1>2
<1>4. QED BY PTL, <1>1, <1>3 DEF ActivationLifecycleSpec

THEOREM InvocationUsesDurableFullSelection ==
  ASSUME ActivationLifecycleInvariant, Hooks!Invoke
  PROVE currentVisible = <<active, plans[active]>> /\ currentStable = currentVisible
  BY SMT, HookDomain, SelectionDomain
  DEF ActivationLifecycleInvariant, LifecycleInvariant, Invariant, TypeOK,
      CurrentProjection, HookSelection, PlanSelectionBinding, Hooks!Invoke,
      Hooks!ActivationInductiveInvariant, Hooks!ProcessInvariant, Hooks!NativePhases,
      Hooks!TypeOK, Hooks!ActivationParameters, Selection!Invariant, Selection!TypeOK,
      Selection!CurrentInvariant, Selection!Bound, Selection!IsTransaction, Selection!Selections

THEOREM ClearedPendingResurrectionHasNoUnsettledIntents ==
  ASSUME ActivationLifecycleInvariant, pendingC = ActivationNone, pendingD # ActivationNone
  PROVE pendingD \in archiveD /\ Hooks!AllSettled(pendingD, stateD)
  BY SMT DEF ActivationLifecycleInvariant, Hooks!ActivationInductiveInvariant,
      Hooks!HomeInvariant, Hooks!ArchiveInvariant, Hooks!TypeOK

=============================================================================
