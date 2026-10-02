---- MODULE PublicationSelectionProofs ----
EXTENDS PublicationBridgeProofs

THEOREM JournalStutterPreservesInvariant ==
  ASSUME Invariant, UNCHANGED vars
  PROVE Invariant'
  BY SMT DEF Invariant, TypeOK, CellInvariantAll, EpochInvariant, MarkerCoversEntries,
      ControlInvariant, CachedEntriesEmpty, StableEntriesEmpty, Committed, Classification,
      vars, selectionVars, control, Selection!Invariant, Selection!TypeOK,
      Selection!ReservationInvariant, Selection!CurrentInvariant, Selection!MarkerInvariant,
      Selection!Bound

THEOREM PublicationCommonNamesPersist ==
  ASSUME Publication!Invariant, Publication!Next
  PROVE stable \intersect visible \subseteq stable' \intersect visible'
  BY SMT, CorrectPublication
  DEF Publication!Next, Publication!Allocate, Publication!WriteArtifact, Publication!SyncArtifact,
      Publication!PublishArtifact, Publication!SealArtifactName, Publication!WriteHighWater,
      Publication!SyncHighWater, Publication!SealStagingDirectory, Publication!RenameGeneration,
      Publication!SealGenerationDirectory, Publication!ProcessDeath, Publication!WritebackWater,
      Publication!WritebackGenerations, Publication!PowerLoss, Publication!Invariant,
      Publication!StorageInvariant

THEOREM LifecycleInitialSafety == LifecycleInit => LifecycleInvariant
<1>1. LifecycleInit => Invariant /\ Publication!Invariant
  BY SMT, JournalInitialSafety, PublicationComponentInitialSafety DEF LifecycleInit
<1>2. LifecycleInit => admittedGenerations \in SUBSET (stable \intersect visible) /\ CurrentNamesDurableGeneration
  BY SMT, SelectionDomain, CompositionDomain
  DEF LifecycleInit, Init, Selection!Init, Selection!Parameters,
      Publication!Init, CurrentNamesDurableGeneration
<1>3. QED BY <1>1, <1>2 DEF LifecycleInvariant

THEOREM LifecycleJournalProjection ==
  ASSUME NEW admitted \in BOOLEAN, LifecycleNext(admitted)
  PROVE [Next(admittedGenerations, admitted)]_vars
  BY SMT DEF LifecycleNext, PublicationStep, AdmitObservedGeneration, JournalStep,
      LifecycleProcessDeath, LifecyclePowerLoss, Next, EnvironmentNext

THEOREM LifecycleComponentSafety ==
  ASSUME NEW admitted \in BOOLEAN, LifecycleInvariant, LifecycleNext(admitted)
  PROVE Invariant' /\ Publication!Invariant'
<1>1. admittedGenerations \in SUBSET GenerationIds
  BY SMT, CompositionDomain
  DEF LifecycleInvariant, Publication!Invariant, Publication!TypeOK
<1>2. [Next(admittedGenerations, admitted)]_vars
  BY LifecycleJournalProjection
<1>3. Invariant'
  BY SMT, <1>1, <1>2, JournalInduction, JournalStutterPreservesInvariant DEF LifecycleInvariant
<1>4. [Publication!Next]_publicationVars
  BY LifecycleProjectsPublication
<1>5. Publication!Invariant'
  BY SMT, <1>4, PublicationComponentInduction, PublicationStutterPreservesInvariant DEF LifecycleInvariant
<1>6. QED BY <1>3, <1>5

THEOREM LifecycleAdmissionPreserved ==
  ASSUME NEW admitted \in BOOLEAN, LifecycleInvariant, LifecycleNext(admitted)
  PROVE admittedGenerations' \in SUBSET (stable' \intersect visible')
<1>1. Publication!Invariant'
  BY SMT, LifecycleComponentSafety
<1>2. [Publication!Next]_publicationVars
  BY LifecycleProjectsPublication
<1>3. stable \intersect visible \subseteq stable' \intersect visible'
  BY SMT, <1>2, PublicationCommonNamesPersist DEF LifecycleInvariant, publicationVars
<1>4. QED
  BY SMT, <1>1, <1>3
  DEF LifecycleInvariant, LifecycleNext, PublicationStep, AdmitObservedGeneration,
      JournalStep, LifecycleProcessDeath, LifecyclePowerLoss, Publication!Invariant,
      Publication!ReturnedPublicationIsStable, Publication!ControlInvariant, Publication!WritebackGenerations

THEOREM JournalChangesCurrentOnlyToAdmittedGenerations ==
  ASSUME NEW admitted \in BOOLEAN, LifecycleInvariant, JournalStep(admitted)
  PROVE CurrentNamesDurableGeneration'
  BY SMT, CorrectJournal
  DEF LifecycleInvariant, CurrentNamesDurableGeneration, JournalStep, CoreNext,
      BeginEpoch, PrepareSelection, WriteRunMarker, SealRunMarker, WriterStep, FinishWriting,
      FinishNoop, FlipSelection, SealCommittedSelection, ClassifyRecovery, RetryRecovery,
      RestoreStep, FinishRestoring, RemoveEntry, SealEntryRemoval, RemoveMarker,
      AlreadyMissingMarker, SealMarkerRemoval, WritebackSelection, WritebackDestination,
      ExternalDestination, UnexpectedSelection, Selection!Reserve, Selection!PublishLink,
      Selection!SealLink, Selection!WriteMarker, Selection!SealMarker, Selection!Flip,
      Selection!SealCurrent, Selection!ClearMarker, Selection!SealReservations,
      Selection!ForeignCurrent, selectionVars, publicationVars

THEOREM LifecycleCurrentPreserved ==
  ASSUME NEW admitted \in BOOLEAN, LifecycleInvariant, LifecycleNext(admitted)
  PROVE CurrentNamesDurableGeneration'
<1>1. [Publication!Next]_publicationVars
  BY LifecycleProjectsPublication
<1>2. stable \intersect visible \subseteq stable' \intersect visible'
  BY SMT, <1>1, PublicationCommonNamesPersist DEF LifecycleInvariant, publicationVars
<1>3. CASE JournalStep(admitted)
  BY SMT, <1>3, JournalChangesCurrentOnlyToAdmittedGenerations
<1>4. CASE PublicationStep \/ (\E generation \in GenerationIds : AdmitObservedGeneration(generation))
  BY SMT, <1>2, <1>4
  DEF LifecycleInvariant, CurrentNamesDurableGeneration, PublicationStep, AdmitObservedGeneration,
      vars, selectionVars
<1>5. CASE LifecycleProcessDeath \/ LifecyclePowerLoss
  BY SMT, <1>2, <1>5
  DEF LifecycleInvariant, CurrentNamesDurableGeneration, LifecycleProcessDeath, LifecyclePowerLoss,
      CrashProcess, CrashPower, Selection!ProcessDeath, Selection!PowerLoss
<1>6. QED BY <1>3, <1>4, <1>5 DEF LifecycleNext

THEOREM LifecycleInduction ==
  ASSUME NEW admitted \in BOOLEAN, LifecycleInvariant, LifecycleNext(admitted)
  PROVE LifecycleInvariant'
  BY LifecycleComponentSafety, LifecycleAdmissionPreserved, LifecycleCurrentPreserved
  DEF LifecycleInvariant

THEOREM PublicationSelectionSafety == LifecycleSpec => []LifecycleInvariant
<1>1. LifecycleInit => LifecycleInvariant
  BY LifecycleInitialSafety
<1>2. LifecycleInvariant /\ [LifecycleNext(TRUE)]_lifecycleVars => LifecycleInvariant'
  BY SMT, LifecycleInduction, JournalStutterPreservesInvariant, PublicationStutterPreservesInvariant
  DEF LifecycleInvariant, CurrentNamesDurableGeneration, lifecycleVars, vars, selectionVars, publicationVars
<1>3. QED BY PTL, <1>1, <1>2 DEF LifecycleSpec

=============================================================================
