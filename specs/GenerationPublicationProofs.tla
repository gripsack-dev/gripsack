---- MODULE GenerationPublicationProofs ----
EXTENDS GenerationPublication, TLAPS

ASSUME CorrectProtocol == Mutant = "none"

THEOREM GenerationInitialSafety == Init => Invariant
  BY SMT, Parameters, CorrectProtocol
  DEF Init, Invariant, TypeOK, StorageInvariant, ControlInvariant,
      ReturnedPublicationIsStable, GenerationIds, Artifacts, Phases

THEOREM GenerationInduction == Invariant /\ Next => Invariant'
<1> SUFFICES ASSUME Invariant, Next PROVE Invariant'
  OBVIOUS
<1> USE DEF Invariant, TypeOK, StorageInvariant, ControlInvariant,
           ReturnedPublicationIsStable, GenerationIds, Artifacts, Phases
<1>1. CASE Allocate
  BY SMT, Parameters, CorrectProtocol, <1>1 DEF Allocate
<1>2. CASE \E artifact \in Artifacts : WriteArtifact(artifact)
  BY SMT, Parameters, CorrectProtocol, <1>2 DEF WriteArtifact, Required
<1>3. CASE \E artifact \in Artifacts : SyncArtifact(artifact)
  BY SMT, Parameters, CorrectProtocol, <1>3 DEF SyncArtifact
<1>4. CASE \E artifact \in Artifacts : PublishArtifact(artifact)
  BY SMT, Parameters, CorrectProtocol, <1>4 DEF PublishArtifact
<1>5. CASE \E artifact \in Artifacts : SealArtifactName(artifact)
  BY SMT, Parameters, CorrectProtocol, <1>5 DEF SealArtifactName
<1>6. CASE WriteHighWater
  BY SMT, Parameters, CorrectProtocol, <1>6 DEF WriteHighWater
<1>7. CASE SyncHighWater
  BY SMT, Parameters, CorrectProtocol, <1>7 DEF SyncHighWater
<1>8. CASE SealStagingDirectory
  BY SMT, Parameters, CorrectProtocol, <1>8 DEF SealStagingDirectory
<1>9. CASE RenameGeneration
  BY SMT, Parameters, CorrectProtocol, <1>9 DEF RenameGeneration
<1>10. CASE SealGenerationDirectory
  BY SMT, Parameters, CorrectProtocol, <1>10 DEF SealGenerationDirectory
<1>11. CASE ProcessDeath
  BY SMT, Parameters, CorrectProtocol, <1>11 DEF ProcessDeath
<1>12. CASE WritebackWater
  BY SMT, Parameters, CorrectProtocol, <1>12 DEF WritebackWater
<1>13. CASE WritebackGenerations
  BY SMT, Parameters, CorrectProtocol, <1>13 DEF WritebackGenerations
<1>14. CASE PowerLoss
  BY SMT, Parameters, CorrectProtocol, <1>14 DEF PowerLoss
<1>15. QED
  BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10,
     <1>11, <1>12, <1>13, <1>14 DEF Next

THEOREM AllocationNeverReusesPublishedId ==
    Invariant /\ Allocate => candidate' \notin everPublished
  BY SMT DEF Invariant, TypeOK, GenerationIds, StorageInvariant, Allocate

THEOREM StableHighWaterNeverDecreases == Invariant /\ Next => waterStable' >= waterStable
  BY SMT, CorrectProtocol
  DEF Invariant, TypeOK, GenerationIds, StorageInvariant, Next, Allocate, WriteArtifact, SyncArtifact,
      PublishArtifact, SealArtifactName, WriteHighWater, SyncHighWater,
      SealStagingDirectory, RenameGeneration, SealGenerationDirectory,
      ProcessDeath, WritebackWater, WritebackGenerations, PowerLoss

THEOREM PublicationReservesWholeHistoryBeforeRename ==
    Invariant /\ RenameGeneration =>
      \A generation \in everPublished \union {candidate} : generation <= waterStable
  BY SMT DEF Invariant, TypeOK, GenerationIds, ControlInvariant, RenameGeneration

THEOREM GenerationSafety == Spec => []Invariant
<1>1. Init => Invariant
  BY GenerationInitialSafety
<1>2. Invariant /\ [Next]_vars => Invariant'
  BY GenerationInduction
  DEF Invariant, TypeOK, StorageInvariant, ControlInvariant, ReturnedPublicationIsStable, vars
<1>3. QED
  BY PTL, <1>1, <1>2 DEF Spec

THEOREM GenerationInvariantImpliesPublication ==
    Invariant => NewPublicationsHaveStableHighWater /\ PublishedHistoryHasDurableWitness /\
                 PublicationSealsEntireHistoryFloor /\ PublishedGenerationHasDurableArtifacts /\
                 ReturnedPublicationIsStable
  BY SMT
  DEF Invariant, StorageInvariant, ControlInvariant, NewPublicationsHaveStableHighWater,
      PublishedHistoryHasDurableWitness, PublicationSealsEntireHistoryFloor,
      PublishedGenerationHasDurableArtifacts

THEOREM DurableGenerationPublication == Spec =>
    [](NewPublicationsHaveStableHighWater /\ PublishedHistoryHasDurableWitness /\
       PublicationSealsEntireHistoryFloor /\ PublishedGenerationHasDurableArtifacts /\ ReturnedPublicationIsStable)
  BY PTL, GenerationSafety, GenerationInvariantImpliesPublication

=============================================================================
