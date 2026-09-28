---- MODULE PublicationBridgeProofs ----
EXTENDS PublicationSelection, JournalMetadataProofs

ASSUME CorrectPublication == PublicationMutant = "none"
PublicationProofs == INSTANCE GenerationRetentionProofs
    WITH phase <- publicationPhase, Mutant <- PublicationMutant

THEOREM PublicationStateNamesAgree ==
    /\ Publication!Init = PublicationProofs!Init
    /\ Publication!Invariant = PublicationProofs!Invariant
    /\ Publication!Invariant' = PublicationProofs!Invariant'
  BY DEF Publication!Init, PublicationProofs!Init,
      Publication!Invariant, PublicationProofs!Invariant,
      Publication!TypeOK, PublicationProofs!TypeOK,
      Publication!StorageInvariant, PublicationProofs!StorageInvariant,
      Publication!ControlInvariant, PublicationProofs!ControlInvariant,
      Publication!ReturnedPublicationIsStable, PublicationProofs!ReturnedPublicationIsStable,
      Publication!GenerationIds, PublicationProofs!GenerationIds,
      Publication!Artifacts, PublicationProofs!Artifacts,
      Publication!Phases, PublicationProofs!Phases, Publication!Required, PublicationProofs!Required

THEOREM PublicationProofPremises ==
    PublicationProofs!Parameters /\ PublicationProofs!CorrectProtocol
  BY SMT, PublicationDomain, CorrectPublication
  DEF Publication!Parameters, PublicationProofs!Parameters, PublicationProofs!CorrectProtocol

THEOREM PublicationTransitionNamesAgree == Publication!Next = PublicationProofs!Next
  BY DEF Publication!Next, PublicationProofs!Next,
      Publication!Allocate, PublicationProofs!Allocate,
      Publication!WriteArtifact, PublicationProofs!WriteArtifact,
      Publication!SyncArtifact, PublicationProofs!SyncArtifact,
      Publication!PublishArtifact, PublicationProofs!PublishArtifact,
      Publication!SealArtifactName, PublicationProofs!SealArtifactName,
      Publication!WriteHighWater, PublicationProofs!WriteHighWater,
      Publication!SyncHighWater, PublicationProofs!SyncHighWater,
      Publication!SealStagingDirectory, PublicationProofs!SealStagingDirectory,
      Publication!RenameGeneration, PublicationProofs!RenameGeneration,
      Publication!SealGenerationDirectory, PublicationProofs!SealGenerationDirectory,
      Publication!ProcessDeath, PublicationProofs!ProcessDeath,
      Publication!WritebackWater, PublicationProofs!WritebackWater,
      Publication!WritebackGenerations, PublicationProofs!WritebackGenerations,
      Publication!PowerLoss, PublicationProofs!PowerLoss,
      Publication!Artifacts, PublicationProofs!Artifacts, Publication!Required, PublicationProofs!Required,
      Publication!GenerationIds, PublicationProofs!GenerationIds

THEOREM PublicationComponentInitialSafety == Publication!Init => Publication!Invariant
  BY SMT, PublicationStateNamesAgree, PublicationProofPremises,
     PublicationProofs!GenerationInitialSafety

THEOREM PublicationComponentInduction == Publication!Invariant /\ Publication!Next => Publication!Invariant'
  BY SMT, PublicationStateNamesAgree, PublicationProofPremises,
     PublicationTransitionNamesAgree, PublicationProofs!GenerationInduction

THEOREM PublicationStutterPreservesInvariant ==
  ASSUME Publication!Invariant, UNCHANGED publicationVars
  PROVE Publication!Invariant'
  BY SMT DEF publicationVars, Publication!Invariant, Publication!TypeOK,
      Publication!StorageInvariant, Publication!ControlInvariant, Publication!ReturnedPublicationIsStable

THEOREM LifecycleProjectsPublication ==
  ASSUME NEW admitted \in BOOLEAN, LifecycleNext(admitted)
  PROVE [Publication!Next]_publicationVars
  BY SMT DEF LifecycleNext, PublicationStep, AdmitObservedGeneration, JournalStep,
      LifecycleProcessDeath, LifecyclePowerLoss, Publication!Next

=============================================================================
