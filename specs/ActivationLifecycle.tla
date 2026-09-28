---- MODULE ActivationLifecycle ----
EXTENDS PublicationSelection

CONSTANTS IntentCount, IntentCounts, MaxAttempts, ActivationNone, ActivationMutant
VARIABLES currentC, currentD, pendingC, pendingD, plans, stateC, stateD,
          archiveC, archiveD, activationPhase, active, cursor, permit, result,
          invokedUnder, returned, terminalEver, replayedTerminal, cleared
Hooks == INSTANCE ActivationInvariant
    WITH TXS <- Transactions, GENS <- GenerationIds, NONE <- ActivationNone,
         MUTANT <- ActivationMutant, phase <- activationPhase
ASSUME HookDomain == Hooks!ActivationParameters /\ Hooks!OptionalValueDomain

hookVars == <<currentC, currentD, pendingC, pendingD, plans, stateC, stateD,
              archiveC, archiveD, activationPhase, active, cursor, permit, result,
              invokedUnder, returned, terminalEver, replayedTerminal, cleared>>
activationLifecycleVars == <<lifecycleVars, hookVars>>
HookSelection(selection) ==
    IF selection = NoSelection THEN ActivationNone
    ELSE IF selection[1] \notin Transactions THEN ActivationNone
    ELSE IF plans[selection[1]] = ActivationNone THEN ActivationNone ELSE selection[1]
NeedsActivation(selection) ==
    IF selection = NoSelection THEN FALSE
    ELSE IF selection[1] \notin Transactions THEN FALSE ELSE IntentCounts[selection[1]] > 0

ActivationLifecycleInit == LifecycleInit /\ Hooks!Init

PublicationWithHooks == PublicationStep /\ UNCHANGED hookVars
ObservedGenerationWithHooks(generation) == AdmitObservedGeneration(generation) /\ UNCHANGED hookVars
BeginLifecycleEpoch(transaction, generation) ==
    /\ activationPhase = "idle" /\ pendingC = ActivationNone
    /\ BeginEpoch(transaction, generation)
    /\ UNCHANGED <<publicationVars, admittedGenerations, hookVars>>

\* These journal actions do not change the shared home current name. New
\* epochs, current publication and home-directory barriers have joint actions.
StationaryJournalStep(admitted) ==
    /\ ((admitted /\ (PrepareSelection \/ WriteRunMarker \/ SealRunMarker
          \/ (\E destination \in Destinations : WriterStep(destination) \/ RestoreStep(destination) \/ RemoveEntry(destination))
          \/ FinishWriting \/ FinishNoop \/ ClassifyRecovery \/ RetryRecovery \/ FinishRestoring
          \/ SealEntryRemoval \/ RemoveMarker \/ AlreadyMissingMarker \/ SealMarkerRemoval))
        \/ ((Selection!SealReservations \/ Selection!SealMarker) /\ UNCHANGED <<cells, control>>)
        \/ (\E destination \in Destinations : WritebackDestination(destination)
              \/ (\E value \in Objects : ExternalDestination(destination, value))))
    /\ UNCHANGED <<publicationVars, admittedGenerations, hookVars>>

PrepareActivationPlan(transaction, generation) ==
    /\ mode = "publishing" /\ pending = <<transaction, generation>>
    /\ IntentCounts[transaction] > 0 /\ generation \in admittedGenerations
    /\ Hooks!Prepare(transaction, generation)
    /\ UNCHANGED lifecycleVars
WriteActivationPointer ==
    /\ mode = "publishing" /\ Hooks!WritePointer
    /\ UNCHANGED lifecycleVars
JournalHomeBarrier == Selection!SealCurrent /\ UNCHANGED <<cells, control>>
SealActivationPointer ==
    /\ mode = "publishing" /\ Hooks!SyncPointer /\ JournalHomeBarrier
    /\ UNCHANGED <<publicationVars, admittedGenerations>>
FlipLifecycleSelection ==
    /\ FlipSelection(admittedGenerations)
    /\ IF NeedsActivation(pending) THEN Hooks!Flip
       ELSE IF currentC = ActivationNone THEN UNCHANGED hookVars ELSE Hooks!SelectWithoutActivation
    /\ UNCHANGED <<publicationVars, admittedGenerations>>

HookCurrentWriteback ==
    IF currentC = currentD THEN UNCHANGED hookVars ELSE Hooks!WritebackCurrent
SealLifecycleCommit ==
    /\ SealCommittedSelection /\ (Hooks!SyncFlip \/ HookCurrentWriteback)
    /\ UNCHANGED <<publicationVars, admittedGenerations>>
WritebackLifecycleCurrent ==
    /\ JournalHomeBarrier /\ HookCurrentWriteback
    /\ UNCHANGED <<publicationVars, admittedGenerations>>

\* Hook effects begin only after the journal's two-barrier drain. Failures do
\* not roll current back; interrupted effects remain Started and replayable.
ActivationExecutionStep ==
    /\ mode = "idle"
    /\ (Hooks!Open \/ Hooks!SealOutcomes \/ Hooks!Authorize \/ Hooks!Skip
        \/ Hooks!Start \/ Hooks!StartBarrier \/ Hooks!Invoke
        \/ (\E verdict \in {"succeeded", "failed"} : Hooks!Return(verdict))
        \/ Hooks!WriteOutcome \/ Hooks!OutcomeBarrier \/ Hooks!Supersede
        \/ Hooks!SkipSuperseded \/ Hooks!SupersedeBarrier \/ Hooks!FinishSuperseding
        \/ Hooks!FinishScan \/ Hooks!WriteArchive \/ Hooks!ArchiveBarrier \/ Hooks!Clear
        \/ Hooks!EarlySuccess)
    /\ UNCHANGED lifecycleVars
ActivationHomeBarrier ==
    /\ mode = "idle"
    /\ (Hooks!SealHome \/ Hooks!ClearBarrier \/ Hooks!SyncFlip \/ Hooks!SealWithoutActivation)
    /\ JournalHomeBarrier /\ UNCHANGED <<publicationVars, admittedGenerations>>
ActivationStorageStep ==
    /\ (Hooks!WritebackPending \/ (\E transaction \in Transactions :
          Hooks!WritebackOutcomes(transaction) \/ Hooks!WritebackArchive(transaction)))
    /\ UNCHANGED lifecycleVars

LifecycleProcessCrash ==
    /\ LifecycleProcessDeath
    /\ IF activationPhase = "idle" THEN UNCHANGED hookVars ELSE Hooks!ProcessDeath
LifecycleStorageCrash ==
    /\ LifecyclePowerLoss /\ currentStable' = currentStable
    /\ (Hooks!PowerLoss \/ (activationPhase = "idle" /\ currentC = currentD /\ pendingC = pendingD
          /\ stateC = stateD /\ archiveC = archiveD /\ UNCHANGED hookVars))

ActivationLifecycleNext(admitted) ==
    \/ PublicationWithHooks \/ (\E generation \in GenerationIds : ObservedGenerationWithHooks(generation))
    \/ (admitted /\ (\E transaction \in Transactions, generation \in GenerationIds :
          BeginLifecycleEpoch(transaction, generation) \/ PrepareActivationPlan(transaction, generation)))
    \/ StationaryJournalStep(admitted)
    \/ (admitted /\ (WriteActivationPointer \/ SealActivationPointer \/ FlipLifecycleSelection
          \/ SealLifecycleCommit \/ ActivationExecutionStep \/ ActivationHomeBarrier))
    \/ WritebackLifecycleCurrent \/ ActivationStorageStep \/ LifecycleProcessCrash \/ LifecycleStorageCrash
ActivationLifecycleSpec == ActivationLifecycleInit /\ [][ActivationLifecycleNext(TRUE)]_activationLifecycleVars

PlanSelectionBinding == \A transaction \in Transactions : plans[transaction] # ActivationNone =>
    /\ transaction \in reservationStable /\ binding[transaction] = plans[transaction]
CurrentProjection == currentC = HookSelection(currentVisible) /\ currentD = HookSelection(currentStable)
PreparationPhases == {"prepared", "pointer-dirty", "pre-flip", "flip-dirty"}
PreparationMatchesEpoch == activationPhase \in PreparationPhases =>
    epochTarget = <<active, plans[active]>>
PreparedPlansHaveWork == \A transaction \in Transactions :
    plans[transaction] # ActivationNone => IntentCounts[transaction] > 0
ActivationLifecycleInvariant ==
    /\ LifecycleInvariant /\ Hooks!ActivationInductiveInvariant
    /\ PlanSelectionBinding /\ CurrentProjection /\ PreparationMatchesEpoch /\ PreparedPlansHaveWork

=============================================================================
