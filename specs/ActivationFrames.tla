---- MODULE ActivationFrames ----
EXTENDS ActivationInvariant, TLAPS

THEOREM ActivationVariableFrame == UNCHANGED vars =>
    /\ UNCHANGED <<currentC, currentD, pendingC, pendingD, plans, stateC, stateD, archiveC, archiveD>>
    /\ UNCHANGED <<phase, active, cursor, permit, result, invokedUnder, returned,
                    terminalEver, replayedTerminal, cleared>>
  BY SMT DEF vars, home, records, archives, process, history

THEOREM ActivationPredicateFrame ==
  ASSUME UNCHANGED vars
  PROVE ActivationInductiveInvariant' = ActivationInductiveInvariant
<1>1. /\ UNCHANGED <<currentC, currentD, pendingC, pendingD, plans, stateC, stateD, archiveC, archiveD>>
      /\ UNCHANGED <<phase, active, cursor, permit, result, invokedUnder, returned,
                      terminalEver, replayedTerminal, cleared>>
  BY ActivationVariableFrame
<1>2. TypeOK' = TypeOK
  BY ONLY SMT, <1>1 DEF TypeOK
<1>3. PlanInvariant' = PlanInvariant
  BY ONLY SMT, <1>1 DEF PlanInvariant, PointerHasPlan
<1>4. HomeInvariant' = HomeInvariant
  BY ONLY SMT, <1>1 DEF HomeInvariant, NoSilentSkip
<1>5. RecordInvariant' = RecordInvariant
  BY ONLY SMT, <1>1 DEF RecordInvariant
<1>6. ArchiveInvariant' = ArchiveInvariant
  BY ONLY SMT, <1>1 DEF ArchiveInvariant
<1>7. HistoryInvariant' = HistoryInvariant
  BY ONLY SMT, <1>1
  DEF HistoryInvariant, OutcomeAfterReturn, TerminalNoReplay, EffectsBindFullSelection,
      InvocationUsesDeclaredIntent
<1>8. ProcessInvariant' = ProcessInvariant
  BY ONLY SMT, <1>1 DEF ProcessInvariant
<1>9. QED
  BY ONLY SMT, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8
  DEF ActivationInductiveInvariant

THEOREM PlanPredicateFrame ==
  ASSUME UNCHANGED <<home, plans, records, archives>>
  PROVE PlanInvariant' = PlanInvariant
  BY SMT DEF PlanInvariant, PointerHasPlan, home, records, archives

THEOREM HomePredicateFrame ==
  ASSUME UNCHANGED <<home, archiveD>>
  PROVE HomeInvariant' = HomeInvariant
  BY SMT DEF HomeInvariant, NoSilentSkip, home

THEOREM RecordPredicateFrame ==
  ASSUME UNCHANGED <<records, pendingC, terminalEver>>
  PROVE RecordInvariant' = RecordInvariant
  BY SMT DEF RecordInvariant, records

THEOREM ArchivePredicateFrame ==
  ASSUME UNCHANGED <<archives, cleared, stateD>>
  PROVE ArchiveInvariant' = ArchiveInvariant
  BY SMT DEF ArchiveInvariant, archives

THEOREM HistoryPredicateFrame ==
  ASSUME UNCHANGED <<records, invokedUnder, returned, replayedTerminal>>
  PROVE HistoryInvariant' = HistoryInvariant
  BY SMT DEF HistoryInvariant, OutcomeAfterReturn, TerminalNoReplay,
      EffectsBindFullSelection, InvocationUsesDeclaredIntent, records

=============================================================================
