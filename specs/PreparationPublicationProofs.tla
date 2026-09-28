---- MODULE PreparationPublicationProofs ----
EXTENDS PreparationPublication, TLAPS

ASSUME CorrectProtocol == Mutant = "none" /\ NamespaceMutant = "none"
NamespaceProofs == INSTANCE NamespaceSealingProofs
    WITH cursor <- namespaceCursor, phase <- namespacePhase, returned <- namespaceReturned,
         Mutant <- NamespaceMutant

THEOREM NamespaceStateNamesAgree ==
    /\ Namespace!Init = NamespaceProofs!Init
    /\ Namespace!Invariant = NamespaceProofs!Invariant
    /\ Namespace!Invariant' = NamespaceProofs!Invariant'
  BY DEF Namespace!Init, NamespaceProofs!Init, Namespace!Invariant, NamespaceProofs!Invariant,
      Namespace!TypeOK, NamespaceProofs!TypeOK, Namespace!ControlInvariant, NamespaceProofs!ControlInvariant,
      Namespace!AuthorityHasDurableNamespace, NamespaceProofs!AuthorityHasDurableNamespace,
      Namespace!AllSealed, NamespaceProofs!AllSealed, Namespace!Names, NamespaceProofs!Names,
      Namespace!Phases, NamespaceProofs!Phases

THEOREM NamespaceProofPremises == NamespaceProofs!Parameters /\ NamespaceProofs!CorrectProtocol
  BY SMT, NamespaceDomain, CorrectProtocol
  DEF Namespace!Parameters, NamespaceProofs!Parameters, NamespaceProofs!CorrectProtocol

THEOREM NamespaceTransitionNamesAgree == Namespace!Next = NamespaceProofs!Next
  BY DEF Namespace!Next, NamespaceProofs!Next, Namespace!SyncDirectory, NamespaceProofs!SyncDirectory,
      Namespace!ReturnAuthority, NamespaceProofs!ReturnAuthority,
      Namespace!ProcessDeath, NamespaceProofs!ProcessDeath, Namespace!ResumeObserved, NamespaceProofs!ResumeObserved,
      Namespace!Writeback, NamespaceProofs!Writeback, Namespace!PowerLoss, NamespaceProofs!PowerLoss,
      Namespace!Names, NamespaceProofs!Names

THEOREM NamespaceInitialComponentSafety == Namespace!Init => Namespace!Invariant
  BY SMT, NamespaceStateNamesAgree, NamespaceProofPremises, NamespaceProofs!NamespaceInitialSafety

THEOREM PreparationNamespaceProjection == Next => [Namespace!Next]_namespaceVars
  BY SMT DEF Next, PublicationStep, Namespace!Next, WriteBytes, SetPrivateMode, SyncFile, PublishName,
      SyncDocumentParent, SealNamespace, ReturnPreparation, WritebackDocuments, WritebackNamespace,
      ProcessDeath, PowerLoss

THEOREM PreparationNamespaceSafety ==
  ASSUME Invariant, Next
  PROVE Namespace!Invariant'
<1>1. [Namespace!Next]_namespaceVars
  BY PreparationNamespaceProjection
<1>2. CASE Namespace!Next
  BY SMT, <1>2, NamespaceStateNamesAgree, NamespaceTransitionNamesAgree,
      NamespaceProofPremises, NamespaceProofs!NamespaceInduction DEF Invariant
<1>3. CASE UNCHANGED namespaceVars
  BY SMT, <1>3 DEF Invariant, namespaceVars, Namespace!Invariant, Namespace!TypeOK,
      Namespace!ControlInvariant, Namespace!AuthorityHasDurableNamespace, Namespace!AllSealed
<1>4. QED BY ONLY SMT, <1>1, <1>2, <1>3

THEOREM PreparationInitialScopeSafety ==
  ASSUME NEW required \in SUBSET Documents, InitFor(required)
  PROVE Invariant
  BY SMT, NamespaceInitialComponentSafety
  DEF InitFor, Invariant, TypeOK, StorageInvariant, ControlInvariant, PreparedDocumentsAreDurable, DocumentPhases

THEOREM PreparationInitialSafety == Init => Invariant
  BY SMT, PreparationInitialScopeSafety DEF Init

THEOREM PreparationResetMatchesInit ==
  ASSUME NEW required \in SUBSET Documents
  PROVE ResetFor(required) = InitFor(required)'
  BY DEF ResetFor, InitFor, Namespace!Reset, Namespace!Init

THEOREM PreparationResetScopeSafety ==
  ASSUME NEW required \in SUBSET Documents, ResetFor(required)
  PROVE Invariant'
  BY SMT, NamespaceDomain, CorrectProtocol
  DEF ResetFor, Invariant, TypeOK, StorageInvariant, ControlInvariant,
      PreparedDocumentsAreDurable, DocumentPhases, Namespace!Reset,
      Namespace!Invariant, Namespace!TypeOK, Namespace!ControlInvariant,
      Namespace!AuthorityHasDurableNamespace, Namespace!AllSealed,
      Namespace!Names, Namespace!Phases, Namespace!Parameters

THEOREM PreparationDocumentInduction == Invariant /\ Next => TypeOK' /\ StorageInvariant' /\ ControlInvariant'
<1> SUFFICES ASSUME Invariant, Next PROVE TypeOK' /\ StorageInvariant' /\ ControlInvariant'
  OBVIOUS
<1> USE DEF Invariant, TypeOK, StorageInvariant, ControlInvariant, DocumentPhases, Next
<1>1. CASE \E document \in Documents : WriteBytes(document)
  BY SMT, CorrectProtocol, <1>1 DEF WriteBytes
<1>2. CASE \E document \in Documents : SetPrivateMode(document)
  BY SMT, CorrectProtocol, <1>2 DEF SetPrivateMode
<1>3. CASE \E document \in Documents : SyncFile(document)
  BY SMT, CorrectProtocol, <1>3 DEF SyncFile
<1>4. CASE \E document \in Documents : PublishName(document)
  BY SMT, CorrectProtocol, <1>4 DEF PublishName
<1>5. CASE \E document \in Documents : SyncDocumentParent(document)
  BY SMT, CorrectProtocol, <1>5 DEF SyncDocumentParent
<1>6. CASE SealNamespace \/ WritebackNamespace
  BY SMT, <1>6 DEF SealNamespace, WritebackNamespace, publicationVars
<1>7. CASE ReturnPreparation
  BY SMT, <1>7 DEF ReturnPreparation
<1>8. CASE WritebackDocuments
  BY SMT, <1>8 DEF WritebackDocuments
<1>9. CASE ProcessDeath
  BY SMT, <1>9 DEF ProcessDeath
<1>10. CASE PowerLoss
  BY SMT, <1>10 DEF PowerLoss
<1>11. QED
  BY ONLY SMT, Next, Invariant, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10
  DEF Next, PublicationStep, Invariant, TypeOK

THEOREM PreparationReturnHasDurableDocuments ==
  ASSUME Invariant, ReturnPreparation
  PROVE requested \subseteq bytesD \intersect modesD \intersect namesD
<1>1. \A document \in requested : document \in Documents /\ documentPhase[document] = "done"
  BY SMT, CorrectProtocol DEF Invariant, TypeOK, ReturnPreparation
<1>2. QED
  BY SMT, <1>1 DEF Invariant, ControlInvariant

THEOREM PreparationDurableSetsGrow ==
  Next => bytesD \subseteq bytesD' /\ modesD \subseteq modesD' /\ namesD \subseteq namesD'
  BY SMT DEF Next, PublicationStep, WriteBytes, SetPrivateMode, SyncFile, PublishName,
      SyncDocumentParent, SealNamespace, ReturnPreparation, WritebackDocuments, WritebackNamespace,
      ProcessDeath, PowerLoss, publicationVars

THEOREM PreparationResultChangesOnlyOnReturn ==
  Next /\ ~ReturnPreparation => UNCHANGED <<returned, requested>>
  BY SMT DEF Next, PublicationStep, WriteBytes, SetPrivateMode, SyncFile, PublishName,
      SyncDocumentParent, SealNamespace, WritebackDocuments, WritebackNamespace,
      ProcessDeath, PowerLoss, publicationVars

THEOREM SealedNamespacePersists ==
  ASSUME Namespace!Invariant, [Namespace!Next]_namespaceVars,
         Namespace!AllSealed, cachedReachable
  PROVE Namespace!AllSealed' /\ cachedReachable'
  BY SMT, NamespaceDomain
  DEF Namespace!Invariant, Namespace!TypeOK, Namespace!AllSealed, Namespace!Names,
      Namespace!Next, Namespace!SyncDirectory, Namespace!ReturnAuthority, Namespace!ProcessDeath,
      Namespace!ResumeObserved, Namespace!Writeback, Namespace!PowerLoss, namespaceVars,
      Namespace!Parameters

THEOREM PreparationDurableAuthorityPreserved ==
  ASSUME Invariant, Next
  PROVE PreparedDocumentsAreDurable'
<1>1. Namespace!Invariant'
  BY PreparationNamespaceSafety
<1>2. returned => Namespace!AllSealed' /\ cachedReachable'
  BY SMT, SealedNamespacePersists, PreparationNamespaceProjection
  DEF Invariant, PreparedDocumentsAreDurable
<1>3. CASE ReturnPreparation
  BY SMT, <1>1, <1>3, PreparationReturnHasDurableDocuments
  DEF PreparedDocumentsAreDurable, ReturnPreparation, Namespace!Invariant, Namespace!AuthorityHasDurableNamespace,
      Next, namespaceVars
<1>4. CASE ~ReturnPreparation
  BY SMT, <1>2, <1>4, PreparationDurableSetsGrow, PreparationResultChangesOnlyOnReturn
  DEF Invariant, PreparedDocumentsAreDurable
<1>5. QED BY ONLY SMT, <1>3, <1>4

THEOREM PreparationInduction == Invariant /\ Next => Invariant'
  BY PreparationDocumentInduction, PreparationNamespaceSafety, PreparationDurableAuthorityPreserved DEF Invariant

THEOREM ArbitraryDocumentPreparationSafety == Spec => []Invariant
<1>1. Init => Invariant BY PreparationInitialSafety
<1>2. Invariant /\ [Next]_vars => Invariant'
  BY SMT, PreparationInduction
  DEF Invariant, TypeOK, StorageInvariant, ControlInvariant, PreparedDocumentsAreDurable,
      Namespace!Invariant, Namespace!TypeOK, Namespace!ControlInvariant,
      Namespace!AuthorityHasDurableNamespace, Namespace!AllSealed, vars, namespaceVars, publicationVars
<1>3. QED BY PTL, <1>1, <1>2 DEF Spec

THEOREM PreparedDocumentAuthority == Spec => []PreparedDocumentsAreDurable
  BY PTL, ArbitraryDocumentPreparationSafety DEF Invariant

=============================================================================
