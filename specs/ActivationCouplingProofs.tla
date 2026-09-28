---- MODULE ActivationCouplingProofs ----
EXTENDS ActivationBridgeProofs

couplingVars == <<plans, binding, reservationStable, currentC, currentD, currentVisible,
                  currentStable, activationPhase, active, epochTarget>>
CouplingInvariant == PlanSelectionBinding /\ CurrentProjection /\ PreparationMatchesEpoch /\ PreparedPlansHaveWork

THEOREM CouplingStateFrame ==
  ASSUME UNCHANGED couplingVars
  PROVE CouplingInvariant' = CouplingInvariant
  BY SMT DEF couplingVars, CouplingInvariant, PlanSelectionBinding, CurrentProjection,
      PreparationMatchesEpoch, PreparedPlansHaveWork, HookSelection

THEOREM PublishingTargetHasDurableReservation ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds,
         Invariant, mode = "publishing", pending = <<transaction, generation>>
  PROVE transaction \in reservationStable /\ binding[transaction] = generation
  BY SMT, SelectionDomain
  DEF Invariant, EpochInvariant, ControlInvariant, Selection!Invariant,
      Selection!MarkerInvariant, Selection!Marker, Selection!Bound, Selection!IsTransaction,
      Selection!Parameters

THEOREM PublishingTargetIsNotCurrent ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds,
         Invariant, mode = "publishing", pending = <<transaction, generation>>
  PROVE /\ (currentVisible # NoSelection => currentVisible[1] # transaction)
        /\ (currentStable # NoSelection => currentStable[1] # transaction)
  BY SMT, SelectionDomain
  DEF Invariant, TypeOK, EpochInvariant, ControlInvariant, Committed,
      Selection!Invariant, Selection!TypeOK, Selection!MarkerInvariant, Selection!Marker,
      Selection!CurrentInvariant, Selection!ReservationInvariant, Selection!Bound,
      Selection!IsTransaction, Selection!Selections, Selection!Parameters

THEOREM NewReservationHasNoSavedPlan ==
  ASSUME NEW transaction \in Transactions, NEW generation \in GenerationIds,
         ActivationLifecycleInvariant, BeginLifecycleEpoch(transaction, generation)
  PROVE plans[transaction] = ActivationNone
  BY SMT DEF ActivationLifecycleInvariant, LifecycleInvariant, Invariant, PlanSelectionBinding,
      BeginLifecycleEpoch, BeginEpoch, Selection!Reserve, Selection!Invariant, Selection!ReservationInvariant

=============================================================================
