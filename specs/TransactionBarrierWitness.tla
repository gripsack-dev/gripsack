---- MODULE TransactionBarrierWitness ----
\* Negative calibration only: checked against the mutated Transaction.tla.
\* Do not import TransactionProofs: its induction theorem is false under
\* this mutation and must never become an assumed fact in this witness.
EXTENDS Transaction, TLAPS

CapturedEntry == [prior |-> StartContent, intended |-> Intended]
CapturedMarker == [prev |-> PREV, target |-> TARGET]
BeforeCleanup == [dest |-> AfterMutate, current |-> TARGET,
                  entry |-> CapturedEntry, marker |-> CapturedMarker]
AfterCleanup == [dest |-> AfterMutate, current |-> TARGET,
                 entry |-> NoEntry, marker |-> CapturedMarker]

ASSUME TagSeparation == NoEntry # CapturedEntry

BadCleanupTransition ==
    /\ phase = "running"
    /\ step = 4
    /\ volatile = BeforeCleanup
    /\ durable = BeforeCleanup
    /\ visible = [dest |-> StartContent, current |-> PREV,
                  entry |-> NoEntry, marker |-> NoMarker]
    /\ beforeRecover = visible
    /\ edited = FALSE
    /\ klass = "none"
    /\ volatile' = AfterCleanup
    /\ durable' = BeforeCleanup
    /\ step' = 5
    /\ phase' = "running"
    /\ UNCHANGED <<visible, edited, klass, beforeRecover>>

\* The actual mutated step admits an unequal durable/volatile journal.
\* The accompanying TLC run separately reaches a violation of Oracle.
THEOREM CleanupBarrierWitness ==
    BadCleanupTransition => DoStep /\ durable' # volatile'
<1> SUFFICES ASSUME BadCleanupTransition PROVE DoStep /\ durable' # volatile'
  OBVIOUS
<1>1. Effect(4, BeforeCleanup) = AfterCleanup
  BY SMT DEF Effect, BeforeCleanup, AfterCleanup, NoEntry
<1>2. DoStep
  <2> HIDE DEF Effect, BeforeCleanup, AfterCleanup
  <2> QED
    BY ONLY SMT, BadCleanupTransition, <1>1 DEF BadCleanupTransition, DoStep
<1>3. durable' # volatile'
  BY SMT, TagSeparation DEF BadCleanupTransition, BeforeCleanup, AfterCleanup
<1>4. QED
  BY <1>2, <1>3
=============================================================================
