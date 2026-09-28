---- MODULE PreparedActivationBridgeProofs ----
EXTENDS PreparedActivationLifecycle, ActivationLifecycleProofs

ASSUME CorrectPreparation == PreparationMutant = "none" /\ PreparationNamespaceMutant = "none"
PreparationProofs == INSTANCE PreparationPublicationProofs
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

THEOREM PreparationStateNamesAgree ==
    /\ Preparation!Invariant = PreparationProofs!Invariant
    /\ Preparation!Invariant' = PreparationProofs!Invariant'
  BY DEF Preparation!Invariant, PreparationProofs!Invariant,
      Preparation!TypeOK, PreparationProofs!TypeOK,
      Preparation!StorageInvariant, PreparationProofs!StorageInvariant,
      Preparation!ControlInvariant, PreparationProofs!ControlInvariant,
      Preparation!PreparedDocumentsAreDurable, PreparationProofs!PreparedDocumentsAreDurable,
      Preparation!DocumentPhases, PreparationProofs!DocumentPhases,
      Preparation!Namespace!Invariant, PreparationProofs!Namespace!Invariant,
      Preparation!Namespace!TypeOK, PreparationProofs!Namespace!TypeOK,
      Preparation!Namespace!ControlInvariant, PreparationProofs!Namespace!ControlInvariant,
      Preparation!Namespace!AuthorityHasDurableNamespace, PreparationProofs!Namespace!AuthorityHasDurableNamespace,
      Preparation!Namespace!AllSealed, PreparationProofs!Namespace!AllSealed,
      Preparation!Namespace!Names, PreparationProofs!Namespace!Names,
      Preparation!Namespace!Phases, PreparationProofs!Namespace!Phases

THEOREM PreparationInitializerNamesAgree ==
  ASSUME NEW required
  PROVE /\ Preparation!InitFor(required) = PreparationProofs!InitFor(required)
        /\ Preparation!ResetFor(required) = PreparationProofs!ResetFor(required)
  BY DEF Preparation!InitFor, PreparationProofs!InitFor, Preparation!ResetFor, PreparationProofs!ResetFor,
      Preparation!Namespace!Init, PreparationProofs!Namespace!Init,
      Preparation!Namespace!Reset, PreparationProofs!Namespace!Reset,
      Preparation!Namespace!Names, PreparationProofs!Namespace!Names

THEOREM PreparationFiniteDomainsAgree ==
    \A values : Preparation!IsFiniteSet(values) = PreparationProofs!IsFiniteSet(values)
  BY DEF Preparation!IsFiniteSet, PreparationProofs!IsFiniteSet

THEOREM PreparationProofPremises ==
    PreparationProofs!Parameters /\ PreparationProofs!NamespaceDomain /\ PreparationProofs!CorrectProtocol
  BY SMT, PreparationDomain, CorrectPreparation, PreparationFiniteDomainsAgree
  DEF Preparation!Parameters, PreparationProofs!Parameters,
      Preparation!NamespaceDomain, PreparationProofs!NamespaceDomain,
      Preparation!Namespace!Parameters, PreparationProofs!Namespace!Parameters,
      PreparationProofs!CorrectProtocol

THEOREM PreparationTransitionNamesAgree == Preparation!Next = PreparationProofs!Next
  BY DEF Preparation!Next, PreparationProofs!Next,
      Preparation!PublicationStep, PreparationProofs!PublicationStep,
      Preparation!WriteBytes, PreparationProofs!WriteBytes,
      Preparation!SetPrivateMode, PreparationProofs!SetPrivateMode,
      Preparation!SyncFile, PreparationProofs!SyncFile,
      Preparation!PublishName, PreparationProofs!PublishName,
      Preparation!SyncDocumentParent, PreparationProofs!SyncDocumentParent,
      Preparation!SealNamespace, PreparationProofs!SealNamespace,
      Preparation!ReturnPreparation, PreparationProofs!ReturnPreparation,
      Preparation!WritebackDocuments, PreparationProofs!WritebackDocuments,
      Preparation!WritebackNamespace, PreparationProofs!WritebackNamespace,
      Preparation!ProcessDeath, PreparationProofs!ProcessDeath,
      Preparation!PowerLoss, PreparationProofs!PowerLoss,
      Preparation!namespaceVars, PreparationProofs!namespaceVars,
      Preparation!publicationVars, PreparationProofs!publicationVars,
      Preparation!Namespace!SyncDirectory, PreparationProofs!Namespace!SyncDirectory,
      Preparation!Namespace!ReturnAuthority, PreparationProofs!Namespace!ReturnAuthority,
      Preparation!Namespace!Writeback, PreparationProofs!Namespace!Writeback,
      Preparation!Namespace!ProcessDeath, PreparationProofs!Namespace!ProcessDeath,
      Preparation!Namespace!PowerLoss, PreparationProofs!Namespace!PowerLoss,
      Preparation!Namespace!Names, PreparationProofs!Namespace!Names

THEOREM PreparationComponentInitialSafety ==
  ASSUME NEW required \in SUBSET PreparationDocuments, Preparation!InitFor(required)
  PROVE Preparation!Invariant
  BY SMT, PreparationProofPremises, PreparationInitializerNamesAgree,
      PreparationStateNamesAgree, PreparationProofs!PreparationInitialScopeSafety

THEOREM PreparationComponentResetSafety ==
  ASSUME NEW required \in SUBSET PreparationDocuments, Preparation!ResetFor(required)
  PROVE Preparation!Invariant'
  BY SMT, PreparationProofPremises, PreparationInitializerNamesAgree,
      PreparationStateNamesAgree, PreparationProofs!PreparationResetScopeSafety

THEOREM PreparationComponentInduction ==
  Preparation!Invariant /\ Preparation!Next => Preparation!Invariant'
  BY SMT, PreparationProofPremises, PreparationStateNamesAgree,
      PreparationTransitionNamesAgree, PreparationProofs!PreparationInduction

THEOREM PreparationComponentFrame ==
  ASSUME Preparation!Invariant, UNCHANGED preparationVars
  PROVE Preparation!Invariant'
  BY SMT DEF preparationVars, Preparation!Invariant, Preparation!TypeOK,
      Preparation!StorageInvariant, Preparation!ControlInvariant, Preparation!PreparedDocumentsAreDurable,
      Preparation!Namespace!Invariant, Preparation!Namespace!TypeOK, Preparation!Namespace!ControlInvariant,
      Preparation!Namespace!AuthorityHasDurableNamespace, Preparation!Namespace!AllSealed

THEOREM PreparedStepsProjectActivationLifecycle ==
  ASSUME NEW admitted \in BOOLEAN, PreparedLifecycleNext(admitted)
  PROVE [ActivationLifecycleNext(admitted)]_activationLifecycleVars
  BY SMT DEF PreparedLifecycleNext, OrdinaryPreparedLifecycleStep, BeginPreparation,
      PreparationIO, FinishPreparation, PreparedProcessCrash, PreparedStorageCrash, ActivationLifecycleNext

THEOREM RequiredDocumentsWithinUniverse ==
  ASSUME NEW transaction \in Transactions
  PROVE RequiredPreparationDocuments(transaction) \subseteq PreparationDocuments
  BY SMT, HookDomain DEF RequiredPreparationDocuments, PreparationDocuments, Hooks!ActivationParameters

=============================================================================
