---- MODULE ObjectNamespaceProofs ----
EXTENDS ObjectNamespacePublication, ObjectPublicationProofs

ASSUME CorrectNamespace == NamespaceMutant = "none"
NamespaceProofs == INSTANCE NamespaceSealingProofs
    WITH Depth <- NamespaceDepth, Mutant <- NamespaceMutant,
         nodes <- namespaceNodes, names <- namespaceNames, cachedReachable <- namespaceReachable,
         cursor <- namespaceCursor, phase <- namespacePhase, returned <- namespaceReturned

THEOREM NamespaceComponentNamesAgree ==
    /\ Namespace!Init = NamespaceProofs!Init
    /\ Namespace!Invariant = NamespaceProofs!Invariant
    /\ Namespace!Invariant' = NamespaceProofs!Invariant'
    /\ Namespace!Next = NamespaceProofs!Next
    /\ ((UNCHANGED Namespace!vars) <=> (UNCHANGED namespaceVars))
    /\ ((UNCHANGED NamespaceProofs!vars) <=> (UNCHANGED namespaceVars))
  BY DEF Namespace!Init, NamespaceProofs!Init, Namespace!Invariant, NamespaceProofs!Invariant,
      Namespace!TypeOK, NamespaceProofs!TypeOK, Namespace!ControlInvariant, NamespaceProofs!ControlInvariant,
      Namespace!AuthorityHasDurableNamespace, NamespaceProofs!AuthorityHasDurableNamespace,
      Namespace!AllSealed, NamespaceProofs!AllSealed, Namespace!Names, NamespaceProofs!Names,
      Namespace!Phases, NamespaceProofs!Phases, Namespace!Next, NamespaceProofs!Next,
      Namespace!SyncDirectory, NamespaceProofs!SyncDirectory, Namespace!ReturnAuthority, NamespaceProofs!ReturnAuthority,
      Namespace!ProcessDeath, NamespaceProofs!ProcessDeath, Namespace!ResumeObserved, NamespaceProofs!ResumeObserved,
      Namespace!Writeback, NamespaceProofs!Writeback, Namespace!PowerLoss, NamespaceProofs!PowerLoss,
      Namespace!vars, NamespaceProofs!vars, namespaceVars

THEOREM NamespaceComponentPremises == NamespaceProofs!Parameters /\ NamespaceProofs!CorrectProtocol
  BY SMT, NamespaceDomain, CorrectNamespace
  DEF Namespace!Parameters, NamespaceProofs!Parameters, NamespaceProofs!CorrectProtocol

THEOREM NamespaceComponentInitialSafety == Namespace!Init => Namespace!Invariant
  BY SMT, NamespaceComponentNamesAgree, NamespaceComponentPremises, NamespaceProofs!NamespaceInitialSafety

THEOREM NamespaceComponentSafety == Namespace!Invariant /\ [Namespace!Next]_namespaceVars => Namespace!Invariant'
  BY SMT, NamespaceComponentNamesAgree, NamespaceComponentPremises, NamespaceProofs!NamespaceInduction
  DEF Namespace!Invariant, Namespace!TypeOK, Namespace!ControlInvariant,
      Namespace!AuthorityHasDurableNamespace, Namespace!AllSealed, namespaceVars

THEOREM ObjectComponentSafety == Invariant /\ [Next]_vars => Invariant'
  BY SMT, PublicationInduction
  DEF Invariant, TypeOK, ControlInvariant, PublishedObjectHasDurablePayload, AuthorityHasDurableObject, vars

THEOREM JoinedPublicationProjectsBothComponents ==
  ObjectNamespaceNext => [Next]_vars /\ [Namespace!Next]_namespaceVars
  BY SMT DEF ObjectNamespaceNext, ObjectWork, NamespaceWork, GrantObjectAuthority,
      ObjectNamespaceProcessDeath, ObjectNamespacePowerLoss, Next, Namespace!Next

THEOREM ObjectNamespaceInitialSafety == ObjectNamespaceInit => ObjectNamespaceInvariant
  BY SMT, PublicationInitialSafety, NamespaceComponentInitialSafety
  DEF ObjectNamespaceInit, ObjectNamespaceInvariant, Init

THEOREM ObjectNamespaceInduction == ObjectNamespaceInvariant /\ ObjectNamespaceNext => ObjectNamespaceInvariant'
<1>1. ObjectNamespaceInvariant /\ ObjectNamespaceNext => Invariant' /\ Namespace!Invariant'
  BY SMT, JoinedPublicationProjectsBothComponents, ObjectComponentSafety, NamespaceComponentSafety
  DEF ObjectNamespaceInvariant
<1>2. ObjectNamespaceInvariant /\ ObjectNamespaceNext => (returned' => namespaceReturned')
  BY SMT DEF ObjectNamespaceInvariant, ObjectNamespaceNext, ObjectWork, NamespaceWork, GrantObjectAuthority,
      ObjectNamespaceProcessDeath, ObjectNamespacePowerLoss, WriteBytes, SetMode, SyncFile, LateMode,
      PublishName, SyncParent, ReturnAuthority, Stop, ObserveAgain, WritebackData, WritebackName, PowerLoss,
      Namespace!SyncDirectory, Namespace!ReturnAuthority, Namespace!ProcessDeath,
      Namespace!ResumeObserved, Namespace!Writeback, Namespace!PowerLoss, vars, namespaceVars
<1>3. QED BY ONLY SMT, <1>1, <1>2 DEF ObjectNamespaceInvariant

THEOREM CompleteObjectPublicationSafety == ObjectNamespaceSpec => []ObjectNamespaceInvariant
<1>1. ObjectNamespaceInit => ObjectNamespaceInvariant BY ObjectNamespaceInitialSafety
<1>2. ObjectNamespaceInvariant /\ [ObjectNamespaceNext]_objectNamespaceVars => ObjectNamespaceInvariant'
  BY SMT, ObjectNamespaceInduction, ObjectComponentSafety, NamespaceComponentSafety
  DEF ObjectNamespaceInvariant, objectNamespaceVars, vars, namespaceVars
<1>3. QED BY PTL, <1>1, <1>2 DEF ObjectNamespaceSpec

THEOREM ReturnedObjectAdmissionSafety == ObjectNamespaceInvariant => ReturnedObjectIsReachableAndDurable
  BY SMT DEF ObjectNamespaceInvariant, Invariant, AuthorityHasDurableObject,
      Namespace!Invariant, Namespace!AuthorityHasDurableNamespace, ReturnedObjectIsReachableAndDurable

THEOREM ObjectReturnDischargesConsumerDurability ==
  ASSUME ObjectNamespaceInvariant, GrantObjectAuthority
  PROVE /\ cached = NewPayload /\ durable = NewPayload /\ cachedName /\ durableName
        /\ Namespace!AllSealed /\ namespaceReachable
  BY SMT, AuthorityRequiresBothBarriers
  DEF ObjectNamespaceInvariant, GrantObjectAuthority, Namespace!Invariant, Namespace!AuthorityHasDurableNamespace

=============================================================================
