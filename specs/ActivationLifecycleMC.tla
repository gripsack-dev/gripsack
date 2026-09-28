---- MODULE ActivationLifecycleMC ----
EXTENDS ActivationLifecycle
CONSTANT first
UniformLifecycleIntents == [transaction \in Transactions |-> IntentCount]
MixedLifecycleIntents == [transaction \in Transactions |-> IF transaction = first THEN 0 ELSE IntentCount]
LegacyInitialSelection == <<Legacy, 0>>
=============================================================================
