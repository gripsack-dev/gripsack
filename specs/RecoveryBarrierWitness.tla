---- MODULE RecoveryBarrierWitness ----
EXTENDS UndoCell, TLAPS

\* Checked only against a private copy whose PriorSealImage has had the
\* destination durability update removed. No safety theorem is imported.
CONSTANTS Original, Changed
ASSUME WitnessDomain == Original \in Objects /\ Changed \in Objects /\ Original # Changed
RestoredButUnsealed ==
    [original |-> Original,
     cached |-> [live |-> Live(Original, "core"), entry |-> Entry(Original, Changed), prior |-> TRUE],
     durable |-> [live |-> Live(Changed, "core"), entry |-> Entry(Original, Changed), prior |-> TRUE],
     control |-> [stage |-> "recover", processed |-> FALSE]]
IncorrectlyAcknowledged == PriorSealImage(RestoredButUnsealed)
Removed == EntryRemovalImage(IncorrectlyAcknowledged)
Drained == EntryWritebackImage(Removed)

THEOREM RestoreBarrierWitnessStartsInSafeState == CellInvariant(RestoredButUnsealed, FALSE)
  BY SMT, WitnessDomain
  DEF RestoredButUnsealed, CellInvariant, CellSpace, DiskSpace, LiveSpace, EntrySpace, ControlSpace,
      Owners, Stages, CellState, WriterState, UndoEvidence, UnsafeOwned, Covered, Live, Entry

THEOREM MissingRestoreBarrierDestroysRecoveryEvidence ==
    /\ CellInvariant(RestoredButUnsealed, FALSE)
    /\ SealPrior(RestoredButUnsealed, IncorrectlyAcknowledged)
    /\ DeleteEntry(IncorrectlyAcknowledged, Removed, FALSE)
    /\ WritebackEntry(Removed, Drained)
    /\ ~Drained.durable.entry.present
    /\ UnsafeOwned(Drained, Drained.durable.live)
  BY SMT, WitnessDomain, RestoreBarrierWitnessStartsInSafeState
  DEF RestoredButUnsealed, IncorrectlyAcknowledged, Removed, Drained,
      SealPrior, PriorSealImage, DeleteEntry, EntryRemovalImage, WritebackEntry, EntryWritebackImage,
      UnsafeOwned, Live, Entry, Missing

=============================================================================
