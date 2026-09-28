---- MODULE PreparedActivationProofs ----
EXTENDS PreparedActivationSteps

THEOREM PreparedLifecycleInitialSafety == PreparedLifecycleInit => PreparedLifecycleInvariant
<1>1. PreparedLifecycleInit => ActivationLifecycleInvariant /\ Preparation!Invariant
  BY SMT, ActivationLifecycleInitialSafety, PreparationComponentInitialSafety DEF PreparedLifecycleInit
<1>2. PreparedLifecycleInit => PreparationHistoryInvariant
  BY SMT DEF PreparedLifecycleInit, ActivationLifecycleInit, Hooks!Init,
      PreparationHistoryInvariant, PreparationTypes, PreparationOwnerBinding,
      EveryPlanHasDurablePreparation, EmptySnapshot, SnapshotSpace
<1>3. QED BY ONLY SMT, <1>1, <1>2 DEF PreparedLifecycleInvariant, PreparationHistoryInvariant

THEOREM PreparedLifecyclePreservesHigherProtocol ==
  ASSUME NEW admitted \in BOOLEAN, PreparedLifecycleInvariant, PreparedLifecycleNext(admitted)
  PROVE ActivationLifecycleInvariant'
  BY SMT, PreparedStepsProjectActivationLifecycle, ActivationLifecycleInduction, ActivationLifecycleStateFrame
  DEF PreparedLifecycleInvariant

THEOREM PreparedStepsProjectPreparation ==
  ASSUME NEW admitted \in BOOLEAN, PreparedLifecycleNext(admitted)
  PROVE (\E transaction \in Transactions : BeginPreparation(transaction)) \/ [Preparation!Next]_preparationVars
  BY SMT DEF PreparedLifecycleNext, OrdinaryPreparedLifecycleStep, PreparationIO,
      FinishPreparation, PreparedProcessCrash, PreparedStorageCrash, Preparation!Next, Preparation!PublicationStep

THEOREM PreparedLifecyclePreservesPreparation ==
  ASSUME NEW admitted \in BOOLEAN, PreparedLifecycleInvariant, PreparedLifecycleNext(admitted)
  PROVE Preparation!Invariant'
<1>1. (\E transaction \in Transactions : BeginPreparation(transaction)) \/ [Preparation!Next]_preparationVars
  BY PreparedStepsProjectPreparation
<1>2. CASE \E transaction \in Transactions : BeginPreparation(transaction)
  BY SMT, <1>2, RequiredDocumentsWithinUniverse, PreparationComponentResetSafety DEF BeginPreparation
<1>3. CASE [Preparation!Next]_preparationVars
  BY SMT, <1>3, PreparationComponentInduction, PreparationComponentFrame DEF PreparedLifecycleInvariant
<1>4. QED BY ONLY SMT, <1>1, <1>2, <1>3

THEOREM PreparedLifecycleHistoryInduction ==
  ASSUME NEW admitted \in BOOLEAN, PreparedLifecycleInvariant, PreparedLifecycleNext(admitted)
  PROVE PreparationHistoryInvariant'
  BY SMT, BeginPreparationPreservesHistory, PreparationIOPreservesHistory,
      FinishPreparationPreservesHistory, OrdinaryPreparedStepPreservesHistory,
      PreparedProcessCrashPreservesHistory, PreparedStorageCrashPreservesHistory
  DEF PreparedLifecycleNext

THEOREM PreparedLifecycleInduction ==
  ASSUME NEW admitted \in BOOLEAN, PreparedLifecycleInvariant, PreparedLifecycleNext(admitted)
  PROVE PreparedLifecycleInvariant'
  BY PreparedLifecyclePreservesHigherProtocol, PreparedLifecyclePreservesPreparation,
      PreparedLifecycleHistoryInduction
  DEF PreparedLifecycleInvariant, PreparationHistoryInvariant

THEOREM GeneralPreparedActivationSafety == PreparedLifecycleSpec => []PreparedLifecycleInvariant
<1>1. PreparedLifecycleInit => PreparedLifecycleInvariant BY PreparedLifecycleInitialSafety
<1>2. PreparedLifecycleInvariant /\ UNCHANGED preparedLifecycleVars => PreparedLifecycleInvariant'
  BY SMT, ActivationLifecycleStateFrame, PreparationComponentFrame
  DEF PreparedLifecycleInvariant, PreparationTypes, PreparationOwnerBinding, EveryPlanHasDurablePreparation,
      preparedLifecycleVars, preparationHistoryVars, preparationVars, activationLifecycleVars,
      lifecycleVars, vars, selectionVars, control, hookVars
<1>3. PreparedLifecycleInvariant /\ [PreparedLifecycleNext(TRUE)]_preparedLifecycleVars => PreparedLifecycleInvariant'
  BY PreparedLifecycleInduction, <1>2
<1>4. QED BY PTL, <1>1, <1>3 DEF PreparedLifecycleSpec

THEOREM PendingPointerRequiresCompleteDurablePreparation ==
  ASSUME PreparedLifecycleInvariant, NEW transaction \in {pendingC, pendingD}, transaction # ActivationNone
  PROVE /\ RequiredPreparationDocuments(transaction) \subseteq preparationHistory[transaction].files
            \intersect preparationHistory[transaction].modes \intersect preparationHistory[transaction].names
        /\ preparationHistory[transaction].namespace
  BY SMT DEF PreparedLifecycleInvariant, ActivationLifecycleInvariant,
      Hooks!ActivationInductiveInvariant, Hooks!TypeOK, Hooks!PlanInvariant, Hooks!PointerHasPlan,
      EveryPlanHasDurablePreparation

THEOREM FreshPreparationNamespacesDoNotOverlap ==
  ASSUME NEW transaction \in Transactions, PreparedLifecycleInvariant, BeginPreparation(transaction)
  PROVE transaction \notin preparationStarted /\
        \A previous \in preparationStarted : \A document, earlier \in PreparationDocuments :
          <<transaction, document>> # <<previous, earlier>>
  BY SMT DEF BeginPreparation

=============================================================================
