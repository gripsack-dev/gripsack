---- MODULE GenerationPublication ----
EXTENDS Integers, FiniteSets

CONSTANTS MaxGeneration, InitialHighWater, InitialGenerations, NeedsProfile, Mutant
ASSUME Parameters ==
    /\ MaxGeneration \in Nat /\ InitialHighWater \in 0..MaxGeneration
    /\ InitialGenerations \in SUBSET (0..MaxGeneration)
    /\ NeedsProfile \in SUBSET (0..MaxGeneration)
    /\ Mutant \in {"none", "high_water", "file_sync", "parent_sync"}

GenerationIds == 0..MaxGeneration
Artifacts == {"manifest", "profile"}
Phases == {"idle", "staging", "water", "stage", "rename", "parent", "published"}
Required(generation) == IF generation \in NeedsProfile THEN Artifacts ELSE {"manifest"}
VARIABLES waterVisible, waterStable, visible, stable, ready, everPublished,
          candidate, allocationFloor, dataVisible, dataStable, namesVisible, namesStable, phase
vars == <<waterVisible, waterStable, visible, stable, ready, everPublished,
          candidate, allocationFloor, dataVisible, dataStable, namesVisible, namesStable, phase>>

\* Initial retained metadata has already passed strict admission/namespace
\* normalization. The stage's capability namespace is supplied by the checked
\* NamespaceSealing protocol. Artifact data/name steps are the projections of
\* ObjectPublication; they deliberately keep file and parent barriers separate.
\* A legacy counter can be below retained history (zero also represents an
\* absent counter). Allocation observes both sources; new publication seals
\* the complete floor before its rename. GC separately preserves that floor
\* before discarding the retained namespace witness.
Init ==
    /\ waterVisible = InitialHighWater /\ waterStable = InitialHighWater
    /\ visible = InitialGenerations /\ stable = InitialGenerations
    /\ ready = InitialGenerations /\ everPublished = InitialGenerations
    /\ candidate = 0 /\ allocationFloor = 0 /\ phase = "idle"
    /\ dataVisible = {} /\ dataStable = {} /\ namesVisible = {} /\ namesStable = {}

Allocate ==
    /\ phase \in {"idle", "published"}
    /\ \E floor \in GenerationIds :
          /\ waterVisible <= floor /\ floor < MaxGeneration
          /\ \A generation \in visible : generation <= floor
          /\ candidate' = floor + 1 /\ allocationFloor' = floor
    /\ phase' = "staging"
    /\ dataVisible' = {} /\ dataStable' = {} /\ namesVisible' = {} /\ namesStable' = {}
    /\ UNCHANGED <<waterVisible, waterStable, visible, stable, ready, everPublished>>
WriteArtifact(artifact) ==
    /\ phase = "staging" /\ artifact \in Required(candidate)
    /\ dataVisible' = dataVisible \union {artifact}
    /\ UNCHANGED <<waterVisible, waterStable, visible, stable, ready, everPublished,
                    candidate, allocationFloor, dataStable, namesVisible, namesStable, phase>>
SyncArtifact(artifact) ==
    /\ phase = "staging" /\ artifact \in dataVisible
    /\ dataStable' = dataStable \union {artifact}
    /\ UNCHANGED <<waterVisible, waterStable, visible, stable, ready, everPublished,
                    candidate, allocationFloor, dataVisible, namesVisible, namesStable, phase>>
PublishArtifact(artifact) ==
    /\ phase = "staging"
    /\ artifact \in IF Mutant = "file_sync" THEN dataVisible ELSE dataStable
    /\ namesVisible' = namesVisible \union {artifact}
    /\ UNCHANGED <<waterVisible, waterStable, visible, stable, ready, everPublished,
                    candidate, allocationFloor, dataVisible, dataStable, namesStable, phase>>
SealArtifactName(artifact) ==
    /\ phase = "staging" /\ artifact \in namesVisible
    /\ namesStable' = namesStable \union {artifact}
    /\ UNCHANGED <<waterVisible, waterStable, visible, stable, ready, everPublished,
                    candidate, allocationFloor, dataVisible, dataStable, namesVisible, phase>>
WriteHighWater ==
    /\ phase = "staging" /\ Required(candidate) \subseteq namesStable
    /\ waterVisible' = candidate /\ phase' = "water"
    /\ UNCHANGED <<waterStable, visible, stable, ready, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable>>
SyncHighWater ==
    /\ phase = "water"
    /\ waterStable' = IF Mutant = "high_water" THEN waterStable ELSE waterVisible
    /\ phase' = "stage"
    /\ UNCHANGED <<waterVisible, visible, stable, ready, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable>>
SealStagingDirectory ==
    /\ phase = "stage"
    /\ ready' = ready \union {candidate} /\ phase' = "rename"
    /\ UNCHANGED <<waterVisible, waterStable, visible, stable, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable>>
RenameGeneration ==
    /\ phase = "rename"
    /\ visible' = visible \union {candidate}
    /\ everPublished' = everPublished \union {candidate} /\ phase' = "parent"
    /\ UNCHANGED <<waterVisible, waterStable, stable, ready, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable>>
SealGenerationDirectory ==
    /\ phase = "parent"
    /\ stable' = IF Mutant = "parent_sync" THEN stable ELSE visible
    /\ phase' = "published"
    /\ UNCHANGED <<waterVisible, waterStable, visible, ready, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable>>

\* Loss of volatile coordinator state abandons staging, not published names.
\* Staging files can remain as ignored orphans; no published ID is recycled.
ProcessDeath ==
    /\ phase' = "idle"
    /\ dataVisible' = {} /\ dataStable' = {} /\ namesVisible' = {} /\ namesStable' = {}
    /\ UNCHANGED <<waterVisible, waterStable, visible, stable, ready, everPublished,
                    candidate, allocationFloor>>
WritebackWater ==
    /\ waterStable' \in {waterVisible, waterStable}
    /\ UNCHANGED <<waterVisible, visible, stable, ready, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable, phase>>
WritebackGenerations ==
    /\ stable' \in {next \in SUBSET (visible \union stable) : visible \intersect stable \subseteq next}
    /\ UNCHANGED <<waterVisible, waterStable, visible, ready, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable, phase>>
PowerLoss ==
    /\ waterStable' \in {waterVisible, waterStable} /\ waterVisible' = waterStable'
    /\ stable' \in {next \in SUBSET (visible \union stable) : visible \intersect stable \subseteq next}
    /\ visible' = stable'
    /\ phase' = "idle"
    /\ dataVisible' = {} /\ dataStable' = {} /\ namesVisible' = {} /\ namesStable' = {}
    /\ UNCHANGED <<ready, everPublished, candidate, allocationFloor>>

\* Retention invokes these primitives only under its lifecycle session and
\* explicit counter/prune barriers. They are not autonomous publication steps.
ObserveAllocationFloor(floor) ==
    /\ phase = "idle" /\ floor \in GenerationIds /\ waterVisible <= floor
    /\ \A generation \in visible : generation <= floor
    /\ waterVisible' = floor
    /\ UNCHANGED <<waterStable, visible, stable, ready, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable, phase>>
SealAllocationFloor ==
    /\ phase = "idle" /\ waterStable' = waterVisible /\ stable' = visible
    /\ UNCHANGED <<waterVisible, visible, ready, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable, phase>>
PruneGeneration(generation) ==
    /\ phase = "idle" /\ generation \in visible /\ visible' = visible \ {generation}
    /\ UNCHANGED <<waterVisible, waterStable, stable, ready, everPublished, candidate,
                    allocationFloor, dataVisible, dataStable, namesVisible, namesStable, phase>>

Next == Allocate \/ (\E artifact \in Artifacts :
          WriteArtifact(artifact) \/ SyncArtifact(artifact) \/ PublishArtifact(artifact) \/ SealArtifactName(artifact))
        \/ WriteHighWater \/ SyncHighWater \/ SealStagingDirectory \/ RenameGeneration
        \/ SealGenerationDirectory \/ ProcessDeath \/ WritebackWater \/ WritebackGenerations \/ PowerLoss
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ waterVisible \in GenerationIds /\ waterStable \in GenerationIds
    /\ candidate \in GenerationIds /\ allocationFloor \in GenerationIds
    /\ visible \in SUBSET GenerationIds /\ stable \in SUBSET GenerationIds
    /\ ready \in SUBSET GenerationIds /\ everPublished \in SUBSET GenerationIds
    /\ dataVisible \in SUBSET Artifacts /\ dataStable \in SUBSET Artifacts
    /\ namesVisible \in SUBSET Artifacts /\ namesStable \in SUBSET Artifacts /\ phase \in Phases
StorageInvariant ==
    /\ waterStable <= waterVisible
    /\ dataStable \subseteq dataVisible /\ namesStable \subseteq namesVisible
    /\ namesVisible \subseteq dataStable
    /\ visible \subseteq everPublished /\ stable \subseteq everPublished /\ everPublished \subseteq ready
    /\ \A generation \in everPublished :
          generation <= waterStable \/ generation \in visible \intersect stable
    /\ \A generation \in ready \ InitialGenerations : generation <= waterStable
ControlInvariant ==
    /\ (phase # "idle" => candidate = allocationFloor + 1)
    /\ (phase = "staging" => waterVisible <= allocationFloor)
    /\ (phase # "idle" => \A generation \in everPublished : generation <= candidate)
    /\ (phase \in {"water", "stage", "rename", "parent", "published"} =>
          /\ waterVisible = candidate /\ Required(candidate) \subseteq namesStable)
    /\ (phase \in {"stage", "rename", "parent", "published"} => waterStable = candidate)
    /\ (phase \in {"rename", "parent", "published"} => candidate \in ready)
    /\ (phase \in {"parent", "published"} => candidate \in visible)
ReturnedPublicationIsStable == phase = "published" => candidate \in stable
Invariant == TypeOK /\ StorageInvariant /\ ControlInvariant /\ ReturnedPublicationIsStable
NewPublicationsHaveStableHighWater ==
    \A generation \in everPublished \ InitialGenerations : generation <= waterStable
PublishedHistoryHasDurableWitness ==
    \A generation \in everPublished : generation <= waterStable \/ generation \in stable
PublicationSealsEntireHistoryFloor ==
    phase \in {"parent", "published"} => \A generation \in everPublished : generation <= waterStable
PublishedGenerationHasDurableArtifacts ==
    phase = "parent" => Required(candidate) \subseteq dataStable /\ Required(candidate) \subseteq namesStable

=============================================================================
