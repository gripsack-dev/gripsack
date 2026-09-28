---- MODULE PublicationSelection ----
EXTENDS JournalLifecycle

CONSTANTS MaxGeneration, InitialHighWater, InitialGenerations, NeedsProfile, PublicationMutant
VARIABLES waterVisible, waterStable, visible, stable, ready, everPublished,
          candidate, allocationFloor, dataVisible, dataStable, namesVisible, namesStable, publicationPhase
Publication == INSTANCE GenerationPublication WITH phase <- publicationPhase, Mutant <- PublicationMutant
ASSUME PublicationDomain == Publication!Parameters
ASSUME CompositionDomain == GenerationIds = Publication!GenerationIds /\ AvailableGenerations = InitialGenerations

VARIABLE admittedGenerations
publicationVars == <<waterVisible, waterStable, visible, stable, ready, everPublished,
                     candidate, allocationFloor, dataVisible, dataStable, namesVisible, namesStable, publicationPhase>>
lifecycleVars == <<vars, publicationVars, admittedGenerations>>
LifecycleInit == Init /\ Publication!Init /\ admittedGenerations = InitialGenerations

\* Availability records successful publication returns or a checked admission
\* barrier, not an oracle that lets the caller inspect stable storage directly.
PublicationStep ==
    /\ (Publication!Allocate
        \/ (\E artifact \in Publication!Artifacts : Publication!WriteArtifact(artifact)
              \/ Publication!SyncArtifact(artifact) \/ Publication!PublishArtifact(artifact)
              \/ Publication!SealArtifactName(artifact))
        \/ Publication!WriteHighWater \/ Publication!SyncHighWater \/ Publication!SealStagingDirectory
        \/ Publication!RenameGeneration \/ Publication!SealGenerationDirectory
        \/ Publication!WritebackWater \/ Publication!WritebackGenerations)
    /\ admittedGenerations' = IF publicationPhase = "parent" /\ publicationPhase' = "published"
          THEN admittedGenerations \union {candidate'} ELSE admittedGenerations
    /\ UNCHANGED vars

\* A cached, complete generation may be observed after an interrupted return.
\* Its artifact bytes/names preceded the rename; this barrier seals the visible
\* generation-name directory before the reader receives selection authority.
AdmitObservedGeneration(generation) ==
    /\ generation \in visible
    /\ Publication!WritebackGenerations /\ stable' = visible
    /\ admittedGenerations' = admittedGenerations \union {generation}
    /\ UNCHANGED vars
JournalStep(admitted) ==
    /\ ((admitted /\ CoreNext(admittedGenerations))
        \/ WritebackSelection
        \/ (\E destination \in Destinations : WritebackDestination(destination)
              \/ (\E value \in Objects : ExternalDestination(destination, value)))
        \/ (\E selection \in Selection!Selections : UnexpectedSelection(admittedGenerations, selection)))
    /\ UNCHANGED <<publicationVars, admittedGenerations>>
LifecycleProcessDeath ==
    /\ CrashProcess /\ Publication!ProcessDeath
    /\ admittedGenerations' = {}
LifecyclePowerLoss ==
    /\ CrashPower /\ Publication!PowerLoss
    /\ admittedGenerations' = {}
LifecycleNext(admitted) == PublicationStep \/ (\E generation \in GenerationIds : AdmitObservedGeneration(generation))
    \/ JournalStep(admitted) \/ LifecycleProcessDeath \/ LifecyclePowerLoss
LifecycleSpec == LifecycleInit /\ [][LifecycleNext(TRUE)]_lifecycleVars

CurrentNamesDurableGeneration == \A selection \in {currentVisible, currentStable} :
    IF selection = NoSelection THEN TRUE ELSE selection[2] \in stable \intersect visible
LifecycleInvariant ==
    /\ Invariant /\ Publication!Invariant
    /\ admittedGenerations \in SUBSET (stable \intersect visible)
    /\ CurrentNamesDurableGeneration

=============================================================================
