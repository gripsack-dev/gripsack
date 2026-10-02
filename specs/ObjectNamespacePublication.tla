---- MODULE ObjectNamespacePublication ----
EXTENDS ObjectPublication

CONSTANTS NamespaceDepth, NamespaceMutant
VARIABLES namespaceNodes, namespaceNames, namespaceReachable, namespaceCursor, namespacePhase, namespaceReturned
Namespace == INSTANCE NamespaceSealing
    WITH Depth <- NamespaceDepth, Mutant <- NamespaceMutant,
         nodes <- namespaceNodes, names <- namespaceNames, cachedReachable <- namespaceReachable,
         cursor <- namespaceCursor, phase <- namespacePhase, returned <- namespaceReturned
ASSUME NamespaceDomain == Namespace!Parameters
namespaceVars == <<namespaceNodes, namespaceNames, namespaceReachable,
                  namespaceCursor, namespacePhase, namespaceReturned>>
objectNamespaceVars == <<vars, namespaceVars>>

ObjectNamespaceInit == Init /\ Namespace!Init
ObjectWork ==
    /\ (WriteBytes \/ SetMode \/ SyncFile \/ LateMode \/ PublishName \/ SyncParent
        \/ ObserveAgain \/ WritebackData \/ WritebackName)
    /\ UNCHANGED namespaceVars
NamespaceWork ==
    /\ (Namespace!SyncDirectory \/ Namespace!ReturnAuthority \/ Namespace!ResumeObserved \/ Namespace!Writeback)
    /\ UNCHANGED vars
GrantObjectAuthority == namespaceReturned /\ ReturnAuthority /\ UNCHANGED namespaceVars
ObjectNamespaceProcessDeath == Stop /\ Namespace!ProcessDeath
ObjectNamespacePowerLoss == PowerLoss /\ Namespace!PowerLoss
ObjectNamespaceNext == ObjectWork \/ NamespaceWork \/ GrantObjectAuthority
    \/ ObjectNamespaceProcessDeath \/ ObjectNamespacePowerLoss
ObjectNamespaceSpec == ObjectNamespaceInit /\ [][ObjectNamespaceNext]_objectNamespaceVars

ObjectNamespaceInvariant == Invariant /\ Namespace!Invariant /\ (returned => namespaceReturned)
ReturnedObjectIsReachableAndDurable == returned =>
    /\ cached = NewPayload /\ durable = NewPayload /\ cachedName /\ durableName
    /\ Namespace!AllSealed /\ namespaceReachable

=============================================================================
