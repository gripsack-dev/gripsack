---- MODULE JournalLifecycle ----
EXTENDS UndoCell

CONSTANTS Destinations, Transactions, GenerationIds, DefaultGeneration, Legacy, NoSelection,
          InitialSelection, AvailableGenerations, SelectionMutant, JournalMutant
ASSUME JournalParameters ==
    /\ IsFiniteSet(Destinations)
    /\ JournalMutant \in {"none", "entry_barrier", "current_barrier", "marker_barrier"}

VARIABLES reservationVisible, reservationStable, linkVisible, linkStable, binding,
          issued, pending, markerVisible, markerStable, currentVisible, currentStable, flipped
Selection == INSTANCE SelectionLifecycle WITH Mutant <- SelectionMutant
ASSUME SelectionDomain == Selection!Parameters
VARIABLES cells, epochPrevious, epochTarget, mode
control == <<epochPrevious, epochTarget, mode>>
selectionVars == <<reservationVisible, reservationStable, linkVisible, linkStable, binding,
                   issued, pending, markerVisible, markerStable, currentVisible, currentStable, flipped>>
vars == <<selectionVars, cells, control>>
Modes == {"idle", "preparing", "marker", "writing", "publishing", "committing",
          "recovering", "restoring", "sealing", "cleanup", "clear-marker", "drain-marker", "refused"}
Committed == epochTarget # NoSelection /\ currentStable = epochTarget
CachedEntriesEmpty == \A destination \in Destinations : ~cells[destination].cached.entry.present
StableEntriesEmpty == \A destination \in Destinations : ~cells[destination].durable.entry.present
Classification == IF markerVisible.present
    THEN Selection!Classify(markerVisible.previous, markerVisible.target, currentVisible)
    ELSE "uncommitted"
CleanupCommitted == Classification = "committed" /\
    (JournalMutant = "current_barrier" \/ currentVisible = currentStable)

Init ==
    /\ Selection!Init
    /\ cells \in [Destinations -> InitialCells]
    /\ epochPrevious = InitialSelection /\ epochTarget = NoSelection /\ mode = "idle"

BeginEpoch(transaction, generation) ==
    /\ mode = "idle" /\ CachedEntriesEmpty /\ StableEntriesEmpty
    /\ Selection!Reserve(transaction, generation)
    /\ cells' = [destination \in Destinations |->
          InitialCell(cells[destination].cached.live.value, cells[destination].durable.live.value)]
    /\ epochPrevious' = currentVisible /\ epochTarget' = <<transaction, generation>>
    /\ mode' = "preparing"
PrepareSelection ==
    /\ mode = "preparing" /\ (Selection!PublishLink \/ Selection!SealLink)
    /\ UNCHANGED <<cells, control>>
WriteRunMarker ==
    /\ mode = "preparing" /\ Selection!WriteMarker /\ mode' = "marker"
    /\ UNCHANGED <<cells, epochPrevious, epochTarget>>
SealRunMarker ==
    /\ mode = "marker"
    /\ IF JournalMutant = "marker_barrier" THEN UNCHANGED selectionVars ELSE Selection!SealMarker
    /\ mode' = "writing" /\ UNCHANGED <<cells, epochPrevious, epochTarget>>

WriterStep(destination) ==
    /\ mode = "writing"
    /\ \E next \in CellImages(cells[destination]) :
          /\ (WritePrior(cells[destination], next) \/ SyncPrior(cells[destination], next)
              \/ (\E value \in Objects : WriteEntry(cells[destination], next, value))
              \/ SyncEntry(cells[destination], next) \/ Mutate(cells[destination], next)
              \/ SyncDestination(cells[destination], next))
          /\ cells' = [cells EXCEPT ![destination] = next]
    /\ UNCHANGED <<selectionVars, control>>
FinishWriting ==
    /\ mode = "writing"
    /\ \A destination \in Destinations : cells[destination].control.stage \in {"capture", "idle"}
    /\ cells' = [destination \in Destinations |-> ProcessDeathImage(cells[destination])]
    /\ mode' = "publishing" /\ UNCHANGED <<selectionVars, epochPrevious, epochTarget>>
FinishNoop ==
    /\ mode = "writing" /\ CachedEntriesEmpty /\ StableEntriesEmpty
    /\ cells' = [destination \in Destinations |-> ProcessDeathImage(cells[destination])]
    /\ mode' = "clear-marker" /\ UNCHANGED <<selectionVars, epochPrevious, epochTarget>>
FlipSelection(available) ==
    /\ mode = "publishing" /\ Selection!Flip(available)
    /\ mode' = "committing" /\ UNCHANGED <<cells, epochPrevious, epochTarget>>
SealCommittedSelection ==
    /\ mode \in {"committing", "sealing"}
    /\ IF JournalMutant = "current_barrier" THEN UNCHANGED selectionVars ELSE Selection!SealCurrent
    /\ mode' = "cleanup" /\ UNCHANGED <<cells, epochPrevious, epochTarget>>

ClassifyRecovery ==
    /\ mode = "recovering"
    /\ mode' = CASE Classification = "committed" -> "sealing"
                 [] Classification = "uncommitted" -> "restoring"
                 [] OTHER -> "refused"
    /\ UNCHANGED <<selectionVars, cells, epochPrevious, epochTarget>>
RetryRecovery ==
    /\ mode = "refused" /\ mode' = "recovering"
    /\ UNCHANGED <<selectionVars, cells, epochPrevious, epochTarget>>
RestoreStep(destination) ==
    /\ mode = "restoring" /\ Classification = "uncommitted"
    /\ \E next \in CellImages(cells[destination]) :
          /\ (Restore(cells[destination], next) \/ SealPrior(cells[destination], next)
              \/ KeepForeign(cells[destination], next))
          /\ cells' = [cells EXCEPT ![destination] = next]
    /\ UNCHANGED <<selectionVars, control>>
FinishRestoring ==
    /\ mode = "restoring" /\ Classification = "uncommitted"
    /\ \A destination \in Destinations :
          cells[destination].cached.entry.present => cells[destination].control.processed
    /\ mode' = "cleanup" /\ UNCHANGED <<selectionVars, cells, epochPrevious, epochTarget>>
RemoveEntry(destination) ==
    /\ mode = "cleanup"
    /\ \E next \in CellImages(cells[destination]) :
          /\ DeleteEntry(cells[destination], next, CleanupCommitted)
          /\ cells' = [cells EXCEPT ![destination] = next]
    /\ UNCHANGED <<selectionVars, control>>
SealEntryRemoval ==
    /\ mode = "cleanup" /\ CachedEntriesEmpty
    /\ Selection!SealMarker
    /\ cells' = IF JournalMutant = "entry_barrier" THEN cells
          ELSE [destination \in Destinations |-> EntryWritebackImage(cells[destination])]
    /\ mode' = "clear-marker" /\ UNCHANGED <<epochPrevious, epochTarget>>
RemoveMarker ==
    /\ mode = "clear-marker" /\ markerVisible.present
    /\ Selection!ClearMarker(IF JournalMutant = "entry_barrier" THEN TRUE ELSE StableEntriesEmpty)
    /\ mode' = "drain-marker" /\ UNCHANGED <<cells, epochPrevious, epochTarget>>
AlreadyMissingMarker ==
    /\ mode = "clear-marker" /\ ~markerVisible.present /\ StableEntriesEmpty
    /\ mode' = "drain-marker" /\ UNCHANGED <<selectionVars, cells, epochPrevious, epochTarget>>
SealMarkerRemoval ==
    /\ mode = "drain-marker" /\ Selection!SealMarker /\ mode' = "idle"
    /\ UNCHANGED <<cells, epochPrevious, epochTarget>>

CrashProcess ==
    /\ Selection!ProcessDeath
    /\ cells' = [destination \in Destinations |-> ProcessDeathImage(cells[destination])]
    /\ mode' = "recovering" /\ UNCHANGED <<epochPrevious, epochTarget>>
CrashPower ==
    /\ Selection!PowerLoss
    /\ cells' \in [Destinations -> UNION {PowerLossImages(cells[d]) : d \in Destinations}]
    /\ \A destination \in Destinations : cells'[destination] \in PowerLossImages(cells[destination])
    /\ mode' = "recovering" /\ UNCHANGED <<epochPrevious, epochTarget>>
WritebackSelection ==
    /\ (Selection!SealReservations \/ Selection!SealMarker \/ Selection!SealCurrent)
    /\ UNCHANGED <<cells, control>>
WritebackDestination(destination) ==
    /\ \E next \in {EntryWritebackImage(cells[destination]), LiveWritebackImage(cells[destination])} :
          cells' = [cells EXCEPT ![destination] = next]
    /\ UNCHANGED <<selectionVars, control>>
ExternalDestination(destination, value) ==
    /\ ExternalEdit(cells[destination], ExternalEditImage(cells[destination], value), value)
    /\ cells' = [cells EXCEPT ![destination] = ExternalEditImage(cells[destination], value)]
    /\ UNCHANGED <<selectionVars, control>>

\* An unexpected admitted third selection is a recovery input, not permission
\* for concurrent or malicious rewriting of private metadata after commitment.
UnexpectedSelection(available, selection) ==
    /\ mode \in {"recovering", "refused"} /\ ~Committed
    /\ currentVisible # epochTarget /\ selection # epochTarget
    /\ Selection!ForeignCurrent(available, selection)
    /\ UNCHANGED <<cells, control>>

CoreNext(available) ==
    \/ (\E transaction \in Transactions, generation \in GenerationIds : BeginEpoch(transaction, generation))
    \/ PrepareSelection \/ WriteRunMarker \/ SealRunMarker
    \/ (\E destination \in Destinations : WriterStep(destination) \/ RestoreStep(destination) \/ RemoveEntry(destination))
    \/ FinishWriting \/ FinishNoop \/ FlipSelection(available) \/ SealCommittedSelection \/ ClassifyRecovery \/ RetryRecovery
    \/ FinishRestoring \/ SealEntryRemoval \/ RemoveMarker \/ AlreadyMissingMarker \/ SealMarkerRemoval
EnvironmentNext(available) ==
    \/ CrashProcess \/ CrashPower \/ WritebackSelection
    \/ (\E destination \in Destinations : WritebackDestination(destination)
          \/ (\E value \in Objects : ExternalDestination(destination, value)))
    \/ (\E selection \in Selection!Selections : UnexpectedSelection(available, selection))
Next(available, admitted) == (admitted /\ CoreNext(available)) \/ EnvironmentNext(available)
Spec == Init /\ [][Next(AvailableGenerations, TRUE)]_vars

TypeOK ==
    /\ Selection!TypeOK /\ cells \in [Destinations -> CellSpace]
    /\ epochPrevious \in Selection!Selections /\ epochTarget \in Selection!Selections /\ mode \in Modes
CellInvariantAll == \A destination \in Destinations : CellInvariant(cells[destination], Committed)
EpochInvariant ==
    /\ (markerVisible.present => markerVisible = Selection!Marker(epochPrevious, epochTarget))
    /\ (markerStable.present => markerStable = Selection!Marker(epochPrevious, epochTarget))
    /\ (pending # NoSelection => pending = epochTarget)
    /\ (epochTarget # NoSelection => Selection!IsTransaction(epochTarget))
    /\ (Committed => currentVisible = epochTarget)
    /\ (currentVisible = epochTarget => markerVisible.present \/ StableEntriesEmpty)
MarkerCoversEntries ==
    /\ (~markerVisible.present => CachedEntriesEmpty /\ StableEntriesEmpty)
    /\ (~markerStable.present => CachedEntriesEmpty /\ StableEntriesEmpty)
ControlInvariant ==
    /\ (mode = "idle" => CachedEntriesEmpty /\ StableEntriesEmpty /\ ~markerVisible.present /\ ~markerStable.present)
    /\ (mode \in {"preparing", "marker"} => CachedEntriesEmpty /\ StableEntriesEmpty /\ ~Committed)
    /\ (mode = "preparing" => pending = epochTarget /\ ~markerVisible.present /\ ~markerStable.present)
    /\ (mode = "marker" => pending = epochTarget /\ markerVisible.present)
    /\ (mode \in {"writing", "publishing", "committing"} =>
          /\ pending = epochTarget /\ markerVisible.present /\ markerStable = markerVisible)
    /\ (mode \in {"preparing", "marker", "writing", "publishing"} =>
          currentVisible = epochPrevious /\ ~Committed)
    /\ (mode = "committing" => currentVisible = epochTarget)
    /\ (mode = "sealing" => Classification = "committed")
    /\ (mode = "restoring" => Classification = "uncommitted" /\
          (~Committed \/ (CachedEntriesEmpty /\ StableEntriesEmpty)))
    /\ (mode = "cleanup" => Classification # "ambiguous" /\
          (Classification = "committed" => currentVisible = currentStable))
    /\ (mode \in {"clear-marker", "drain-marker"} => CachedEntriesEmpty /\ StableEntriesEmpty)
    /\ (mode = "drain-marker" => ~markerVisible.present)
    /\ (mode \notin {"idle", "preparing", "marker", "writing"} =>
          \A destination \in Destinations : cells[destination].control.stage = "recover")
Invariant == TypeOK /\ Selection!Invariant /\ CellInvariantAll /\ EpochInvariant /\ MarkerCoversEntries /\ ControlInvariant
RecoveryEvidencePreserved == ~Committed =>
    \A destination \in Destinations :
      ~cells[destination].durable.entry.present => ~UnsafeOwned(cells[destination], cells[destination].durable.live)
MutationHasDurableMarker == mode = "writing" =>
    \A destination \in Destinations :
      UnsafeOwned(cells[destination], cells[destination].cached.live) => markerStable.present
CleanupHasDurableRestoration == mode = "cleanup" /\ ~Committed =>
    \A destination \in Destinations : ~UnsafeOwned(cells[destination], cells[destination].durable.live)

ExactCommitIdentity == markerVisible.present =>
    (Classification = "committed" <=> currentVisible = markerVisible.target)

=============================================================================
