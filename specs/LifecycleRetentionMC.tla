---- MODULE LifecycleRetentionMC ----
EXTENDS LifecycleRetention
CONSTANTS oldPayload, newPayload, priorPayload
UniformRetentionIntents == [transaction \in Transactions |-> IntentCount]
InitialLegacySelection == <<Legacy, 0>>
GenerationReferences == [generation \in GenerationIds |-> IF generation = 0 THEN {oldPayload} ELSE {newPayload}]
OriginalPriorRoots == [value \in Objects |-> IF value = Absent THEN {} ELSE {priorPayload}]
=============================================================================
