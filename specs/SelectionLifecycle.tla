---- MODULE SelectionLifecycle ----
EXTENDS Integers, FiniteSets

CONSTANTS Transactions, GenerationIds, DefaultGeneration, Legacy, NoSelection,
          InitialSelection, AvailableGenerations, Mutant
Selections == {NoSelection} \union ((Transactions \union {Legacy}) \X GenerationIds)
ASSUME Parameters ==
    /\ IsFiniteSet(Transactions) /\ IsFiniteSet(GenerationIds)
    /\ DefaultGeneration \in GenerationIds /\ Legacy \notin Transactions
    /\ NoSelection \notin ((Transactions \union {Legacy}) \X GenerationIds)
    /\ InitialSelection \in Selections
    /\ AvailableGenerations \in SUBSET GenerationIds
    /\ InitialSelection # NoSelection => InitialSelection[2] \in AvailableGenerations
    /\ Mutant \in {"none", "generation_commit"}

IsTransaction(selection) == IF selection = NoSelection THEN FALSE ELSE selection[1] \in Transactions
InitialTransactions == IF IsTransaction(InitialSelection) THEN {InitialSelection[1]} ELSE {}
MarkerSpace == [present: BOOLEAN, previous: Selections, target: Selections]
EmptyMarker == [present |-> FALSE, previous |-> NoSelection, target |-> NoSelection]
Marker(previous, target) == [present |-> TRUE, previous |-> previous, target |-> target]
SameGeneration(left, right) ==
    IF left = NoSelection \/ right = NoSelection THEN FALSE ELSE left[2] = right[2]
Classify(previous, target, current) ==
    IF current = target \/
       (Mutant = "generation_commit" /\ SameGeneration(current, target))
    THEN "committed" ELSE IF current = previous THEN "uncommitted" ELSE "ambiguous"

VARIABLES reservationVisible, reservationStable, linkVisible, linkStable, binding,
          issued, pending, markerVisible, markerStable, currentVisible, currentStable, flipped
vars == <<reservationVisible, reservationStable, linkVisible, linkStable, binding,
          issued, pending, markerVisible, markerStable, currentVisible, currentStable, flipped>>

Init ==
    /\ reservationVisible = InitialTransactions /\ reservationStable = InitialTransactions
    /\ linkVisible = InitialTransactions /\ linkStable = InitialTransactions /\ issued = InitialTransactions
    /\ binding = [transaction \in Transactions |->
          IF transaction \in InitialTransactions THEN InitialSelection[2] ELSE DefaultGeneration]
    /\ pending = NoSelection /\ markerVisible = EmptyMarker /\ markerStable = EmptyMarker
    /\ currentVisible = InitialSelection /\ currentStable = InitialSelection /\ flipped = FALSE

Bound(selection) ==
    IF ~IsTransaction(selection) THEN TRUE
    ELSE /\ selection[1] \in reservationStable /\ selection[1] \in linkStable
         /\ binding[selection[1]] = selection[2]

Reserve(transaction, generation) ==
    /\ pending = NoSelection /\ ~markerVisible.present /\ ~markerStable.present
    /\ currentVisible = currentStable /\ transaction \notin reservationVisible
    /\ pending' = <<transaction, generation>>
    /\ reservationVisible' = reservationVisible \union {transaction}
    /\ binding' = [binding EXCEPT ![transaction] = generation] /\ flipped' = FALSE
    /\ UNCHANGED <<reservationStable, linkVisible, linkStable, issued,
                    markerVisible, markerStable, currentVisible, currentStable>>
PublishLink ==
    /\ pending # NoSelection
    /\ linkVisible' = linkVisible \union {pending[1]}
    /\ UNCHANGED <<reservationVisible, reservationStable, linkStable, binding, issued,
                    pending, markerVisible, markerStable, currentVisible, currentStable, flipped>>
SealLink ==
    /\ pending # NoSelection /\ pending[1] \in linkVisible
    /\ linkStable' = linkStable \union {pending[1]}
    /\ UNCHANGED <<reservationVisible, reservationStable, linkVisible, binding, issued,
                    pending, markerVisible, markerStable, currentVisible, currentStable, flipped>>
SealReservations ==
    /\ reservationStable' = reservationVisible
    /\ UNCHANGED <<reservationVisible, linkVisible, linkStable, binding, issued,
                    pending, markerVisible, markerStable, currentVisible, currentStable, flipped>>
WriteMarker ==
    /\ pending # NoSelection /\ ~markerVisible.present /\ ~markerStable.present
    /\ Bound(pending) /\ pending[1] \notin issued
    /\ markerVisible' = Marker(currentVisible, pending)
    /\ issued' = issued \union {pending[1]}
    /\ UNCHANGED <<reservationVisible, reservationStable, linkVisible, linkStable, binding,
                    pending, markerStable, currentVisible, currentStable, flipped>>
SealMarker ==
    /\ markerStable' = markerVisible
    /\ UNCHANGED <<reservationVisible, reservationStable, linkVisible, linkStable, binding, issued,
                    pending, markerVisible, currentVisible, currentStable, flipped>>
Flip(available) ==
    /\ pending # NoSelection /\ markerVisible.present /\ markerStable = markerVisible
    /\ markerVisible.target = pending /\ pending[2] \in available
    /\ currentVisible' = pending /\ flipped' = TRUE
    /\ UNCHANGED <<reservationVisible, reservationStable, linkVisible, linkStable, binding, issued,
                    pending, markerVisible, markerStable, currentStable>>
SealCurrent ==
    /\ currentStable' = currentVisible
    /\ UNCHANGED <<reservationVisible, reservationStable, linkVisible, linkStable, binding, issued,
                    pending, markerVisible, markerStable, currentVisible, flipped>>
ClearMarker(entriesDurablyEmpty) ==
    /\ entriesDurablyEmpty /\ markerVisible.present
    /\ Classify(markerVisible.previous, markerVisible.target, currentVisible) # "ambiguous"
    /\ Classify(markerVisible.previous, markerVisible.target, currentVisible) = "committed" =>
         currentVisible = currentStable
    /\ markerVisible' = EmptyMarker /\ pending' = NoSelection
    /\ UNCHANGED <<reservationVisible, reservationStable, linkVisible, linkStable, binding, issued,
                    markerStable, currentVisible, currentStable, flipped>>
ProcessDeath ==
    /\ pending' = NoSelection
    /\ UNCHANGED <<reservationVisible, reservationStable, linkVisible, linkStable, binding, issued,
                    markerVisible, markerStable, currentVisible, currentStable, flipped>>
PowerLoss ==
    /\ reservationStable' \in {next \in SUBSET reservationVisible : reservationStable \subseteq next}
    /\ reservationVisible' = reservationStable'
    /\ linkStable' \in {next \in SUBSET (linkVisible \intersect reservationStable') :
                          linkStable \intersect reservationStable' \subseteq next}
    /\ linkVisible' = linkStable'
    /\ markerStable' \in {markerVisible, markerStable} /\ markerVisible' = markerStable'
    /\ currentStable' \in {currentVisible, currentStable} /\ currentVisible' = currentStable'
    /\ pending' = NoSelection /\ UNCHANGED <<binding, issued, flipped>>

\* A valid but foreign current is an admitted recovery observation after the
\* writer is gone, not an unbounded concurrent-writer/CAS promise.
ForeignCurrent(available, selection) ==
    /\ pending = NoSelection /\ selection \in Selections /\ Bound(selection)
    /\ IF selection = NoSelection THEN TRUE ELSE selection[2] \in available
    /\ currentVisible' = selection
    /\ UNCHANGED <<reservationVisible, reservationStable, linkVisible, linkStable, binding, issued,
                    pending, markerVisible, markerStable, currentStable, flipped>>

CoreNext(available, entriesDurablyEmpty) ==
    \/ (\E transaction \in Transactions : \E generation \in GenerationIds : Reserve(transaction, generation))
    \/ PublishLink \/ SealLink \/ WriteMarker \/ Flip(available) \/ ClearMarker(entriesDurablyEmpty)
StorageNext == SealReservations \/ SealMarker \/ SealCurrent \/ ProcessDeath \/ PowerLoss
Next(available, entriesDurablyEmpty, admitted) ==
    \/ (admitted /\ CoreNext(available, entriesDurablyEmpty))
    \/ StorageNext \/ (\E selection \in Selections : ForeignCurrent(available, selection))
Spec == Init /\ [][Next(AvailableGenerations, TRUE, TRUE)]_vars

TypeOK ==
    /\ reservationVisible \in SUBSET Transactions /\ reservationStable \in SUBSET Transactions
    /\ linkVisible \in SUBSET Transactions /\ linkStable \in SUBSET Transactions /\ issued \in SUBSET Transactions
    /\ binding \in [Transactions -> GenerationIds] /\ pending \in Selections
    /\ markerVisible \in MarkerSpace /\ markerStable \in MarkerSpace
    /\ currentVisible \in Selections /\ currentStable \in Selections /\ flipped \in BOOLEAN
ReservationInvariant ==
    /\ reservationStable \subseteq reservationVisible
    /\ linkStable \subseteq linkVisible /\ linkVisible \subseteq reservationVisible
    /\ issued \subseteq reservationStable /\ issued \subseteq linkStable
    /\ (pending # NoSelection =>
          /\ IsTransaction(pending) /\ pending[1] \in reservationVisible
          /\ binding[pending[1]] = pending[2])
    /\ (pending # NoSelection /\ ~markerVisible.present =>
          pending # currentVisible /\ pending # currentStable)
MarkerInvariant(marker) == marker.present =>
    /\ IsTransaction(marker.target) /\ marker.target[1] \in issued
    /\ marker.target # marker.previous /\ Bound(marker.target) /\ Bound(marker.previous)
CurrentInvariant == Bound(currentVisible) /\ Bound(currentStable)
Invariant == TypeOK /\ ReservationInvariant /\ CurrentInvariant
             /\ MarkerInvariant(markerVisible) /\ MarkerInvariant(markerStable)
PredecessorCannotCommit ==
    markerVisible.present /\ currentVisible = markerVisible.previous =>
      Classify(markerVisible.previous, markerVisible.target, currentVisible) = "uncommitted"

=============================================================================
