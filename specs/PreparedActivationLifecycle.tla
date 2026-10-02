---- MODULE PreparedActivationLifecycle ----
EXTENDS ActivationLifecycle

CONSTANTS PreparationDepth, PreparationMutant, PreparationNamespaceMutant
PreparationDocuments == 0..IntentCount
RequiredPreparationDocuments(transaction) == 0..IntentCounts[transaction]
VARIABLES preparationNodes, preparationNames, preparationReachable,
          preparationNamespaceCursor, preparationNamespacePhase, preparationNamespaceReturned,
          preparationBytesC, preparationBytesD, preparationModesC, preparationModesD,
          preparationNamesC, preparationNamesD, preparationDocumentPhase,
          preparationStopped, preparationReturned, preparationRequested
Preparation == INSTANCE PreparationPublication
    WITH Documents <- PreparationDocuments, Depth <- PreparationDepth,
         Mutant <- PreparationMutant, NamespaceMutant <- PreparationNamespaceMutant,
         nodes <- preparationNodes, names <- preparationNames, cachedReachable <- preparationReachable,
         namespaceCursor <- preparationNamespaceCursor, namespacePhase <- preparationNamespacePhase,
         namespaceReturned <- preparationNamespaceReturned,
         bytesC <- preparationBytesC, bytesD <- preparationBytesD,
         modesC <- preparationModesC, modesD <- preparationModesD,
         namesC <- preparationNamesC, namesD <- preparationNamesD,
         documentPhase <- preparationDocumentPhase, stopped <- preparationStopped,
         returned <- preparationReturned, requested <- preparationRequested
ASSUME PreparationDomain == Preparation!Parameters /\ Preparation!NamespaceDomain

preparationVars == <<preparationNodes, preparationNames, preparationReachable,
    preparationNamespaceCursor, preparationNamespacePhase, preparationNamespaceReturned,
    preparationBytesC, preparationBytesD, preparationModesC, preparationModesD,
    preparationNamesC, preparationNamesD, preparationDocumentPhase,
    preparationStopped, preparationReturned, preparationRequested>>
SnapshotSpace == [files: SUBSET PreparationDocuments, modes: SUBSET PreparationDocuments,
                  names: SUBSET PreparationDocuments, namespace: BOOLEAN]
EmptySnapshot == [files |-> {}, modes |-> {}, names |-> {}, namespace |-> FALSE]
DurablePreparationSnapshot == [files |-> preparationBytesD, modes |-> preparationModesD,
    names |-> preparationNamesD, namespace |-> (Preparation!Namespace!AllSealed /\ preparationReachable)]
VARIABLES preparationOwner, preparationStarted, preparationHistory
preparationHistoryVars == <<preparationOwner, preparationStarted, preparationHistory>>
preparedLifecycleVars == <<activationLifecycleVars, preparationVars, preparationHistoryVars>>

PreparedLifecycleInit ==
    /\ ActivationLifecycleInit /\ Preparation!InitFor({})
    /\ preparationOwner = ActivationNone /\ preparationStarted = {}
    /\ preparationHistory = [transaction \in Transactions |-> EmptySnapshot]

\* One constructor runs at a time under the lifecycle session. A fresh reserved
\* transaction names a new private directory; completed snapshots belong to
\* different immutable namespaces and are not reset with the next constructor.
BeginPreparation(transaction) ==
    /\ preparationOwner = ActivationNone /\ transaction \notin preparationStarted
    /\ mode = "publishing" /\ pending # NoSelection /\ pending[1] = transaction
    /\ pending[2] \in admittedGenerations /\ IntentCounts[transaction] > 0
    /\ activationPhase = "idle" /\ pendingC = ActivationNone /\ plans[transaction] = ActivationNone
    /\ Preparation!ResetFor(RequiredPreparationDocuments(transaction))
    /\ preparationOwner' = transaction /\ preparationStarted' = preparationStarted \union {transaction}
    /\ UNCHANGED <<activationLifecycleVars, preparationHistory>>
PreparationIO ==
    /\ preparationOwner # ActivationNone
    /\ Preparation!Next /\ ~Preparation!ProcessDeath /\ ~Preparation!PowerLoss
    /\ UNCHANGED <<activationLifecycleVars, preparationHistoryVars>>
FinishPreparation(transaction, generation) ==
    /\ preparationOwner = transaction /\ preparationReturned
    /\ PrepareActivationPlan(transaction, generation)
    /\ preparationHistory' = [preparationHistory EXCEPT ![transaction] = DurablePreparationSnapshot]
    /\ preparationOwner' = ActivationNone
    /\ UNCHANGED <<preparationVars, preparationStarted>>

OrdinaryPreparedLifecycleStep(admitted) ==
    /\ (PublicationWithHooks \/ (\E generation \in GenerationIds : ObservedGenerationWithHooks(generation))
        \/ (admitted /\ (\E transaction \in Transactions, generation \in GenerationIds :
              BeginLifecycleEpoch(transaction, generation)))
        \/ StationaryJournalStep(admitted)
        \/ (admitted /\ (WriteActivationPointer \/ SealActivationPointer \/ FlipLifecycleSelection
              \/ SealLifecycleCommit \/ ActivationExecutionStep \/ ActivationHomeBarrier))
        \/ WritebackLifecycleCurrent \/ ActivationStorageStep)
    /\ UNCHANGED <<preparationVars, preparationHistoryVars>>
PreparedProcessCrash ==
    /\ LifecycleProcessCrash /\ Preparation!ProcessDeath /\ UNCHANGED preparationRequested
    /\ preparationOwner' = ActivationNone /\ UNCHANGED <<preparationStarted, preparationHistory>>
PreparedStorageCrash ==
    /\ LifecycleStorageCrash /\ Preparation!PowerLoss /\ UNCHANGED preparationRequested
    /\ preparationOwner' = ActivationNone /\ UNCHANGED <<preparationStarted, preparationHistory>>

PreparedLifecycleNext(admitted) ==
    \/ OrdinaryPreparedLifecycleStep(admitted)
    \/ (admitted /\ ((\E transaction \in Transactions : BeginPreparation(transaction))
          \/ PreparationIO \/ (\E transaction \in Transactions, generation \in GenerationIds :
                FinishPreparation(transaction, generation))))
    \/ PreparedProcessCrash \/ PreparedStorageCrash
PreparedLifecycleSpec == PreparedLifecycleInit /\ [][PreparedLifecycleNext(TRUE)]_preparedLifecycleVars

PreparationTypes ==
    /\ preparationOwner \in Transactions \union {ActivationNone}
    /\ preparationStarted \subseteq Transactions /\ preparationHistory \in [Transactions -> SnapshotSpace]
PreparationOwnerBinding == preparationOwner # ActivationNone =>
    /\ preparationOwner \in preparationStarted
    /\ IntentCounts[preparationOwner] > 0 /\ pendingC = ActivationNone
    /\ preparationRequested = RequiredPreparationDocuments(preparationOwner)
    /\ mode = "publishing" /\ pending # NoSelection /\ pending[1] = preparationOwner
    /\ activationPhase = "idle" /\ plans[preparationOwner] = ActivationNone
    /\ ~preparationStopped
EveryPlanHasDurablePreparation == \A transaction \in Transactions : plans[transaction] # ActivationNone =>
    /\ transaction \in preparationStarted
    /\ RequiredPreparationDocuments(transaction) \subseteq preparationHistory[transaction].files
        \intersect preparationHistory[transaction].modes \intersect preparationHistory[transaction].names
    /\ preparationHistory[transaction].namespace
PreparedLifecycleInvariant == ActivationLifecycleInvariant /\ Preparation!Invariant /\ PreparationTypes
    /\ PreparationOwnerBinding /\ EveryPlanHasDurablePreparation

=============================================================================
