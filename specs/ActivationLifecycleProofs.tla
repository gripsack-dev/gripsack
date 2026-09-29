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
<1>1. /\ Selection!TypeOK /\ Selection!CurrentInvariant
       /\ PlanSelectionBinding /\ CurrentProjection /\ Hooks!ProcessInvariant
  BY ONLY SMT, ActivationLifecycleInvariant
  DEF ActivationLifecycleInvariant, LifecycleInvariant, Invariant,
      Selection!Invariant, Hooks!ActivationInductiveInvariant
<1>2. activationPhase = "permitted"
  BY ONLY SMT, Hooks!Invoke DEF Hooks!Invoke
<1>3. /\ active \in Transactions /\ plans[active] # ActivationNone
       /\ currentC = active /\ currentD = active
  BY ONLY SMT, <1>1, <1>2 DEF Hooks!ProcessInvariant, Hooks!NativePhases
<1>4. active # ActivationNone
  BY ONLY SMT, <1>3, HookDomain DEF HookDomain, Hooks!ActivationParameters
<1>5. /\ currentVisible # NoSelection /\ currentStable # NoSelection
       /\ currentVisible[1] = active /\ currentStable[1] = active
  BY ONLY SMT, <1>1, <1>3, <1>4 DEF CurrentProjection, HookSelection
<1>6. /\ currentVisible = <<currentVisible[1], currentVisible[2]>>
       /\ currentStable = <<currentStable[1], currentStable[2]>>
  BY ONLY SMT, <1>1, <1>5 DEF Selection!TypeOK, Selection!Selections
<1>7. binding[active] = currentVisible[2] /\ binding[active] = currentStable[2]
  BY ONLY SMT, <1>1, <1>3, <1>5
  DEF Selection!CurrentInvariant, Selection!Bound, Selection!IsTransaction
<1>8. binding[active] = plans[active]
  BY ONLY SMT, <1>1, <1>3 DEF PlanSelectionBinding
<1>9. QED BY ONLY SMT, <1>5, <1>6, <1>7, <1>8

THEOREM ClearedPendingResurrectionHasNoUnsettledIntents ==
  ASSUME ActivationLifecycleInvariant, pendingC = ActivationNone, pendingD # ActivationNone
  PROVE pendingD \in archiveD /\ Hooks!AllSettled(pendingD, stateD)
  BY SMT DEF ActivationLifecycleInvariant, Hooks!ActivationInductiveInvariant,
      Hooks!HomeInvariant, Hooks!ArchiveInvariant, Hooks!TypeOK

=============================================================================
