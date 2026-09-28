---- MODULE ActivationRecordProofs ----
EXTENDS ActivationFrames, FunctionFrames

THEOREM OutcomeStateFields ==
  ASSUME NEW outcome \in States
  PROVE /\ outcome.kind \in {"pending", "started", "succeeded", "failed", "superseded"}
        /\ outcome.attempt \in 0..MaxAttempts
        /\ (outcome.kind = "pending" => outcome.attempt = 0)
        /\ (outcome.kind \in {"started", "succeeded", "failed"} => outcome.attempt \in Attempts)
        /\ outcome = State(outcome.kind, outcome.attempt)
  BY SMT, ActivationParameters DEF States, State, Pending, Attempts

THEOREM AttemptOutcomeConstructor ==
  ASSUME NEW kind \in {"started", "succeeded", "failed"}, NEW attempt \in Attempts
  PROVE State(kind, attempt) \in States
  BY SMT DEF State, States

THEOREM SupersededOutcomeConstructor ==
  ASSUME NEW attempt \in 0..MaxAttempts
  PROVE State("superseded", attempt) \in States
  BY SMT DEF State, States

THEOREM OutcomeSlotReplacementType ==
  ASSUME NEW table \in [TXS -> [Intents -> States]],
         NEW transaction \in TXS, NEW intent \in Intents, NEW outcome \in States
  PROVE [table EXCEPT ![transaction][intent] = outcome] \in [TXS -> [Intents -> States]]
  BY SMT, ReplacementDomain DEF ReplaceCell

THEOREM SettledDomain ==
  ASSUME NEW transaction \in TXS, NEW states
  PROVE Settled(transaction, states) \subseteq Pairs
  BY SMT DEF Settled, Pair, Pairs

THEOREM SettledMembership ==
  ASSUME NEW transaction \in TXS, NEW owner \in TXS, NEW intent \in Intents, NEW states
  PROVE Pair(transaction, intent) \in Settled(owner, states) <=>
          transaction = owner /\ Terminal(states[owner][intent])
  BY SMT DEF Settled, Pair

THEOREM OutcomeBarrierRecords ==
  ASSUME NEW transaction \in TXS, TypeOK, RecordInvariant,
         stateD' = [stateD EXCEPT ![transaction] = stateC[transaction]],
         terminalEver' = terminalEver \cup Settled(transaction, stateC),
         UNCHANGED <<stateC, pendingC>>
  PROVE RecordInvariant'
<1>6. /\ stateD' \in [TXS -> [Intents -> States]]
      /\ stateC' = stateC
      /\ stateD'[transaction] = stateC[transaction]
      /\ \A owner \in TXS \ {transaction} : stateD'[owner] = stateD[owner]
  BY SMT, ReplacementDomain, ReplacementFrame DEF TypeOK, ReplaceCell
<1>1. \A owner \in TXS, intent \in Intents :
        /\ stateD'[owner][intent].attempt <= stateC'[owner][intent].attempt
        /\ (Terminal(stateD'[owner][intent]) => stateC'[owner][intent] = stateD'[owner][intent])
  BY SMT, OutcomeStateFields, ActivationParameters
  DEF TypeOK, RecordInvariant, Attempts
<1>2. \A owner \in TXS, intent \in Intents :
        Pair(owner, intent) \in terminalEver' <=> Terminal(stateD'[owner][intent])
  BY SMT, SettledMembership DEF TypeOK, RecordInvariant
<1>3. \A owner \in TXS : owner # pendingC' => stateC'[owner] = stateD'[owner]
  BY SMT DEF TypeOK, RecordInvariant
<1>4. \A owner \in TXS, first, second \in Intents :
        stateC'[owner][first] # stateD'[owner][first] /\
        stateC'[owner][second] # stateD'[owner][second] => first = second
  <2> TAKE owner \in TXS, first \in Intents, second \in Intents
  <2>1. CASE owner = transaction
    BY ONLY SMT, <1>6, <2>1
  <2>2. CASE owner # transaction
    <3>1. /\ stateD'[owner] = stateD[owner] /\ stateC'[owner] = stateC[owner]
      BY SMT, <1>6, <2>2 DEF TypeOK
    <3>2. stateC[owner][first] # stateD[owner][first] /\
            stateC[owner][second] # stateD[owner][second] => first = second
      BY DEF RecordInvariant
    <3>3. QED
      BY ONLY SMT, <3>1, <3>2
  <2>3. QED BY <2>1, <2>2
<1>5. QED
  BY <1>1, <1>2, <1>3, <1>4 DEF RecordInvariant

THEOREM PowerLossRecords ==
  ASSUME TypeOK, RecordInvariant,
         stateC' = stateD, UNCHANGED <<stateD, terminalEver>>
  PROVE RecordInvariant'
  BY SMT, OutcomeStateFields, ActivationParameters
  DEF TypeOK, RecordInvariant, Attempts

THEOREM OutcomeSlotReplacementFrame ==
  ASSUME NEW table \in [TXS -> [Intents -> States]],
         NEW transaction \in TXS, NEW intent \in Intents, NEW outcome \in States
  PROVE /\ [table EXCEPT ![transaction][intent] = outcome][transaction][intent] = outcome
        /\ \A owner \in TXS \ {transaction} :
             [table EXCEPT ![transaction][intent] = outcome][owner] = table[owner]
        /\ \A other \in Intents \ {intent} :
             [table EXCEPT ![transaction][intent] = outcome][transaction][other] = table[transaction][other]
  BY SMT, ReplacementFrame DEF ReplaceCell

THEOREM DirtyOutcomeRecords ==
  ASSUME NEW transaction \in TXS, NEW intent \in Intents, NEW outcome \in States,
         TypeOK, RecordInvariant, transaction = pendingC,
         stateC[transaction] = stateD[transaction],
         ~Terminal(stateD[transaction][intent]),
         stateD[transaction][intent].attempt <= outcome.attempt,
         stateC' = [stateC EXCEPT ![transaction][intent] = outcome],
         UNCHANGED <<stateD, pendingC, terminalEver>>
  PROVE RecordInvariant'
<1>1. /\ stateC'[transaction][intent] = outcome
      /\ \A owner \in TXS \ {transaction} : stateC'[owner] = stateC[owner]
      /\ \A other \in Intents \ {intent} : stateC'[transaction][other] = stateC[transaction][other]
  BY SMT, OutcomeSlotReplacementFrame DEF TypeOK
<1>2. \A owner \in TXS, index \in Intents :
        /\ stateD'[owner][index].attempt <= stateC'[owner][index].attempt
        /\ (Terminal(stateD'[owner][index]) => stateC'[owner][index] = stateD'[owner][index])
        /\ (Pair(owner, index) \in terminalEver' <=> Terminal(stateD'[owner][index]))
  BY SMT, <1>1 DEF RecordInvariant
<1>3. \A owner \in TXS : owner # pendingC' => stateC'[owner] = stateD'[owner]
  BY SMT, <1>1 DEF RecordInvariant
<1>4. \A owner \in TXS, first, second \in Intents :
        stateC'[owner][first] # stateD'[owner][first] /\
        stateC'[owner][second] # stateD'[owner][second] => first = second
  BY SMT, <1>1 DEF RecordInvariant
<1>5. QED BY <1>2, <1>3, <1>4 DEF RecordInvariant

THEOREM OutcomeWriteHistory ==
  ASSUME NEW transaction \in TXS, NEW intent \in Intents, NEW outcome \in States,
         TypeOK, HistoryInvariant,
         outcome.kind \in {"succeeded", "failed"} => Triple(transaction, intent, outcome.attempt) \in returned,
         stateC' = [stateC EXCEPT ![transaction][intent] = outcome],
         UNCHANGED <<stateD, invokedUnder, returned, replayedTerminal>>
  PROVE HistoryInvariant'
<1>1. /\ stateC'[transaction][intent] = outcome
      /\ \A owner \in TXS \ {transaction} : stateC'[owner] = stateC[owner]
      /\ \A other \in Intents \ {intent} : stateC'[transaction][other] = stateC[transaction][other]
  BY SMT, OutcomeSlotReplacementFrame DEF TypeOK
<1>2. OutcomeAfterReturn'
  BY SMT, <1>1 DEF HistoryInvariant, OutcomeAfterReturn
<1>3. QED
  BY SMT, <1>2
  DEF HistoryInvariant, TerminalNoReplay, EffectsBindFullSelection, InvocationUsesDeclaredIntent

THEOREM OutcomeSealHistory ==
  ASSUME NEW transaction \in TXS, TypeOK, HistoryInvariant,
         stateD' = [stateD EXCEPT ![transaction] = stateC[transaction]],
         UNCHANGED <<stateC, invokedUnder, returned, replayedTerminal>>
  PROVE HistoryInvariant'
  BY SMT, ReplacementFrame
  DEF ReplaceCell, TypeOK, HistoryInvariant, OutcomeAfterReturn, TerminalNoReplay,
      EffectsBindFullSelection, InvocationUsesDeclaredIntent

THEOREM OutcomeSealArchive ==
  ASSUME NEW transaction \in TXS, TypeOK, RecordInvariant, ArchiveInvariant,
         stateD' = [stateD EXCEPT ![transaction] = stateC[transaction]],
         archiveD \subseteq archiveD', UNCHANGED <<archiveC, cleared>>
  PROVE ArchiveInvariant'
  BY SMT, ReplacementFrame, ActivationParameters
  DEF ReplaceCell, TypeOK, RecordInvariant, ArchiveInvariant, AllSettled, DeclaredIntents, Intents

THEOREM OutcomeWritePlan ==
  ASSUME NEW transaction \in TXS, NEW intent \in DeclaredIntents(transaction), NEW outcome \in States,
         TypeOK, PlanInvariant, plans[transaction] # NONE,
         stateC' = [stateC EXCEPT ![transaction][intent] = outcome],
         UNCHANGED <<home, plans, stateD, archives>>
  PROVE PlanInvariant'
  BY SMT, ActivationParameters, OutcomeSlotReplacementFrame
  DEF TypeOK, PlanInvariant, PointerHasPlan, home, archives, DeclaredIntents, Intents

THEOREM OutcomeSealPlan ==
  ASSUME NEW transaction \in TXS, TypeOK, PlanInvariant,
         stateD' = [stateD EXCEPT ![transaction] = stateC[transaction]],
         archiveD' \subseteq archiveC, UNCHANGED <<home, plans, stateC, archiveC>>
  PROVE PlanInvariant'
  BY SMT, ReplacementFrame
  DEF ReplaceCell, TypeOK, PlanInvariant, PointerHasPlan, home

THEOREM PowerLossHistory ==
  ASSUME HistoryInvariant, stateC' = stateD,
         UNCHANGED <<stateD, invokedUnder, returned, replayedTerminal>>
  PROVE HistoryInvariant'
  BY SMT DEF HistoryInvariant, OutcomeAfterReturn, TerminalNoReplay,
      EffectsBindFullSelection, InvocationUsesDeclaredIntent

=============================================================================
