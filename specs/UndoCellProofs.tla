---- MODULE UndoCellProofs ----
EXTENDS UndoCell, TLAPS

THEOREM LiveType ==
  ASSUME NEW value \in Objects, NEW owner \in Owners
  PROVE Live(value, owner) \in LiveSpace
  BY SMT DEF Live, LiveSpace

THEOREM MissingType ==
  ASSUME NEW original \in Objects
  PROVE Missing(original) \in EntrySpace
  BY SMT DEF Missing, EntrySpace

THEOREM PresentEntryType ==
  ASSUME NEW before \in Objects, NEW after \in Objects
  PROVE Entry(before, after) \in EntrySpace
  BY SMT DEF Entry, EntrySpace

THEOREM LiveProjection ==
  ASSUME NEW value, NEW owner
  PROVE /\ Live(value, owner).value = value /\ Live(value, owner).owner = owner
  BY SMT DEF Live

THEOREM EntryProjection ==
  ASSUME NEW before, NEW after
  PROVE /\ Entry(before, after).present = TRUE
        /\ Entry(before, after).before = before
        /\ Entry(before, after).after = after
  BY SMT DEF Entry

THEOREM MissingProjection ==
  ASSUME NEW original
  PROVE Missing(original).present = FALSE
  BY SMT DEF Missing

THEOREM DiskLiveUpdateType ==
  ASSUME NEW disk \in DiskSpace, NEW live \in LiveSpace
  PROVE [disk EXCEPT !.live = live] \in DiskSpace
  BY SMT DEF DiskSpace

THEOREM ControlStageUpdateType ==
  ASSUME NEW control \in ControlSpace, NEW stage \in Stages
  PROVE [control EXCEPT !.stage = stage] \in ControlSpace
  BY SMT DEF ControlSpace

THEOREM CachedControlUpdateType ==
  ASSUME NEW c \in CellSpace, NEW cached \in DiskSpace, NEW control \in ControlSpace
  PROVE [c EXCEPT !.cached = cached, !.control = control] \in CellSpace
  BY SMT DEF CellSpace

THEOREM DiskEntryUpdateType ==
  ASSUME NEW disk \in DiskSpace, NEW entry \in EntrySpace
  PROVE [disk EXCEPT !.entry = entry] \in DiskSpace
  BY SMT DEF DiskSpace

THEOREM ControlProcessedUpdateType ==
  ASSUME NEW control \in ControlSpace, NEW processed \in BOOLEAN
  PROVE [control EXCEPT !.processed = processed] \in ControlSpace
  BY SMT DEF ControlSpace

THEOREM BothDisksUpdateType ==
  ASSUME NEW c \in CellSpace, NEW cached \in DiskSpace, NEW durable \in DiskSpace
  PROVE [c EXCEPT !.cached = cached, !.durable = durable] \in CellSpace
  BY SMT DEF CellSpace

THEOREM CellValueTypes ==
  ASSUME NEW c \in CellSpace
  PROVE /\ c.original \in Objects
        /\ c.cached.live.value \in Objects
        /\ c.durable.live.value \in Objects
        /\ c.cached.entry.before \in Objects
        /\ c.cached.entry.after \in Objects
  BY SMT DEF CellSpace, DiskSpace, LiveSpace, EntrySpace

THEOREM InitialDiskType ==
  ASSUME NEW original \in Objects
  PROVE InitialDisk(original) \in DiskSpace
<1>1. Live(original, "initial") \in LiveSpace
  BY SMT, LiveType DEF Owners
<1>2. Missing(original) \in EntrySpace
  BY MissingType
<1>3. QED
  BY SMT, <1>1, <1>2 DEF InitialDisk, DiskSpace

THEOREM InitialCellType ==
  ASSUME NEW original \in Objects, NEW stableOriginal \in Objects
  PROVE InitialCell(original, stableOriginal) \in CellSpace
<1>1. InitialDisk(original) \in DiskSpace
  BY InitialDiskType
<1>2. Live(stableOriginal, "initial") \in LiveSpace
  BY SMT, LiveType DEF Owners
<1>3. [InitialDisk(original) EXCEPT !.live = Live(stableOriginal, "initial")] \in DiskSpace
  BY ONLY SMT, <1>1, <1>2 DEF DiskSpace
<1>4. QED
  BY SMT, <1>1, <1>3 DEF InitialCell, CellSpace, ControlSpace, Stages

THEOREM InitialCellFields ==
  ASSUME NEW original \in Objects, NEW stableOriginal \in Objects
  PROVE /\ InitialCell(original, stableOriginal).original = original
        /\ InitialCell(original, stableOriginal).cached.live = Live(original, "initial")
        /\ InitialCell(original, stableOriginal).durable.live = Live(stableOriginal, "initial")
        /\ InitialCell(original, stableOriginal).cached.entry = Missing(original)
        /\ InitialCell(original, stableOriginal).durable.entry = Missing(original)
        /\ InitialCell(original, stableOriginal).cached.prior = FALSE
        /\ InitialCell(original, stableOriginal).durable.prior = FALSE
        /\ InitialCell(original, stableOriginal).control.stage = "capture"
        /\ InitialCell(original, stableOriginal).control.processed = FALSE
  BY SMT DEF InitialCell, InitialDisk

THEOREM InitialCellSafety ==
  ASSUME NEW original \in Objects, NEW stableOriginal \in Objects, NEW committed \in BOOLEAN
  PROVE CellInvariant(InitialCell(original, stableOriginal), committed)
  BY SMT, InitialCellType, InitialCellFields
  DEF CellInvariant, CellState, Live, Missing, WriterState, UndoEvidence, UnsafeOwned, Covered

THEOREM InitialCellSetSafety ==
  ASSUME NEW cell \in InitialCells, NEW committed \in BOOLEAN
  PROVE CellInvariant(cell, committed)
  BY SMT, InitialCellSafety DEF InitialCells

USE DEF DiskSpace, ControlSpace, CellState
USE DEF PriorWriteImage, PriorSyncImage, EntryWriteImage, EntrySyncImage,
        MutationImage, DestinationSyncImage, ExternalEditImage,
        ProcessDeathImage, EntryWritebackImage, LiveWritebackImage,
        PowerLossImage, RestorationImage, PriorSealImage, KeepImage, EntryRemovalImage

THEOREM PriorWriteSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), WritePrior(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, WritePrior

THEOREM PriorSyncSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), SyncPrior(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, SyncPrior

THEOREM EntryWriteSafety ==
  ASSUME NEW c, NEW next, NEW value \in Objects, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), WriteEntry(c, next, value)
  PROVE CellInvariant(next, committed)
<1>1. Entry(c.cached.live.value, value) \in EntrySpace
  BY SMT, PresentEntryType, CellValueTypes DEF CellInvariant
<1>2. next \in CellSpace
  BY SMT, <1>1, DiskEntryUpdateType, ControlStageUpdateType, CachedControlUpdateType
  DEF CellInvariant, CellSpace, Stages, WriteEntry
<1>3. CellState(next, committed)
  BY SMT, EntryProjection, LiveProjection
  DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
      UndoEvidence, UnsafeOwned, Covered, WriteEntry
<1>4. QED
  BY <1>2, <1>3 DEF CellInvariant

THEOREM EntrySyncSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), SyncEntry(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, SyncEntry

THEOREM MutationSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), Mutate(c, next)
  PROVE CellInvariant(next, committed)
<1>1. next \in CellSpace
  <2>1. Live(c.cached.entry.after, "core") \in LiveSpace
    BY SMT, LiveType DEF CellInvariant, CellSpace, EntrySpace, Owners
  <2>2. QED
    BY SMT, <2>1, DiskLiveUpdateType, ControlStageUpdateType, CachedControlUpdateType
    DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, Mutate
<1>2. CellState(next, committed)
  BY SMT, LiveProjection
  DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
      UndoEvidence, UnsafeOwned, Covered, Mutate
<1>3. QED
  BY <1>1, <1>2 DEF CellInvariant

THEOREM DestinationSyncSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), SyncDestination(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, SyncDestination

THEOREM ExternalEditSafety ==
  ASSUME NEW c, NEW next, NEW value \in Objects, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), ExternalEdit(c, next, value)
  PROVE CellInvariant(next, committed)
<1>1. Live(value, "external") \in LiveSpace
  BY SMT, LiveType DEF Owners
<1>2. next \in CellSpace
  BY SMT, <1>1, DiskLiveUpdateType, BothDisksUpdateType
  DEF CellInvariant, CellSpace, ExternalEdit
<1>3. CellState(next, committed)
  BY SMT, LiveProjection
  DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
      UndoEvidence, UnsafeOwned, Covered, ExternalEdit
<1>4. QED
  BY <1>2, <1>3 DEF CellInvariant

THEOREM ProcessDeathSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), ProcessDeath(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, ProcessDeath

THEOREM EntryWritebackSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), WritebackEntry(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, WritebackEntry

THEOREM LiveWritebackSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), WritebackLive(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, WritebackLive

THEOREM PowerLossSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), PowerLoss(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, PowerLoss

THEOREM RestorationSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), Restore(c, next)
  PROVE CellInvariant(next, committed)
<1>1. Live(c.original, "core") \in LiveSpace
  BY SMT, LiveType, CellValueTypes DEF CellInvariant, Owners
<1>2. next \in CellSpace
  BY SMT, <1>1, DiskLiveUpdateType, ControlProcessedUpdateType, CachedControlUpdateType
  DEF CellInvariant, CellSpace, Restore
<1>3. CellState(next, committed)
  BY SMT, LiveProjection
  DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
      UndoEvidence, UnsafeOwned, Covered, Restore
<1>4. QED
  BY <1>2, <1>3 DEF CellInvariant

THEOREM SealPriorSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), SealPrior(c, next)
  PROVE CellInvariant(next, committed)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, SealPrior

THEOREM KeepForeignSafety ==
  ASSUME NEW c, NEW next,
         CellInvariant(c, FALSE), KeepForeign(c, next)
  PROVE CellInvariant(next, FALSE)
  BY SMT DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
             UndoEvidence, UnsafeOwned, Covered, Live, Foreign, KeepForeign

THEOREM EntryRemovalSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), DeleteEntry(c, next, committed)
  PROVE CellInvariant(next, committed)
<1>1. Missing(c.original) \in EntrySpace
  BY SMT, MissingType, CellValueTypes DEF CellInvariant
<1>2. next \in CellSpace
  BY SMT, <1>1, DiskEntryUpdateType, CachedControlUpdateType
  DEF CellInvariant, CellSpace, DeleteEntry
<1>3. CellState(next, committed)
  BY SMT, MissingProjection
  DEF CellInvariant, CellSpace, EntrySpace, Owners, Stages, WriterState,
      UndoEvidence, UnsafeOwned, Covered, DeleteEntry
<1>4. QED
  BY <1>2, <1>3 DEF CellInvariant

THEOREM CommitMonotonicity ==
  ASSUME NEW c, CellInvariant(c, FALSE)
  PROVE CellInvariant(c, TRUE)
  BY SMT DEF CellInvariant, UndoEvidence

THEOREM CellStepSafety ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), CellStep(c, next, committed)
  PROVE CellInvariant(next, committed)
  BY SMT, PriorWriteSafety, PriorSyncSafety, EntryWriteSafety, EntrySyncSafety,
     MutationSafety, DestinationSyncSafety, ExternalEditSafety,
     ProcessDeathSafety, EntryWritebackSafety, LiveWritebackSafety,
     PowerLossSafety, RestorationSafety, SealPriorSafety, KeepForeignSafety,
     EntryRemovalSafety
  DEF CellStep, CoreCellStep

THEOREM RequiredEvidenceBeforeMutation ==
  ASSUME NEW c, NEW next, NEW committed \in BOOLEAN,
         CellInvariant(c, committed), Mutate(c, next)
  PROVE /\ c.durable.prior /\ c.durable.entry.present
        /\ c.durable.entry = c.cached.entry
  BY SMT DEF CellInvariant, WriterState, Mutate

THEOREM NoLostUndoEvidence ==
  ASSUME NEW c, CellInvariant(c, FALSE), ~c.durable.entry.present
  PROVE ~UnsafeOwned(c, c.durable.live)
  BY SMT DEF CellInvariant, UndoEvidence, Covered

THEOREM SuccessorEnumerationComplete ==
  ASSUME NEW c, NEW next, NEW committed,
         CellStep(c, next, committed)
  PROVE next \in CellSuccessors(c, committed)
<1> HIDE DEF PriorWriteImage, PriorSyncImage, EntryWriteImage, EntrySyncImage,
             MutationImage, DestinationSyncImage, ExternalEditImage,
             ProcessDeathImage, EntryWritebackImage, LiveWritebackImage,
             PowerLossImage, RestorationImage, PriorSealImage, KeepImage, EntryRemovalImage
<1> SUFFICES next \in CellImages(c)
  BY DEF CellSuccessors
<1>1. CASE \E value \in Objects : WriteEntry(c, next, value)
  BY SMT, <1>1 DEF CellImages, WriteEntry
<1>2. CASE \E value \in Objects : ExternalEdit(c, next, value)
  BY SMT, <1>2 DEF CellImages, ExternalEdit
<1>3. CASE PowerLoss(c, next)
  \* A single tuple bound retains the full independent Cartesian choice
  \* while avoiding the pinned backends' unsupported multi-bound set image.
  <2>1. PICK entry \in {c.cached.entry, c.durable.entry},
             live \in {c.cached.live, c.durable.live},
             prior \in {c.cached.prior, c.durable.prior} :
          next = PowerLossImage(c, entry, live, prior)
    BY <1>3 DEF PowerLoss
  <2>2. <<entry, live, prior>> \in
          {c.cached.entry, c.durable.entry}
            \X {c.cached.live, c.durable.live}
            \X {c.cached.prior, c.durable.prior}
    BY SMT, <2>1
  <2>3. next = PowerLossImage(c, <<entry, live, prior>>[1],
                                 <<entry, live, prior>>[2], <<entry, live, prior>>[3])
    BY SMT, <2>1
  <2>4. next \in PowerLossImages(c)
    BY SMT, <2>2, <2>3 DEF PowerLossImages
  <2>5. QED
    BY SMT, <2>4 DEF CellImages
<1>4. CASE /\ ~(\E value \in Objects : WriteEntry(c, next, value))
           /\ ~(\E value \in Objects : ExternalEdit(c, next, value))
           /\ ~PowerLoss(c, next)
  BY SMT, <1>4
  DEF CellImages, CellStep, CoreCellStep, WritePrior, SyncPrior,
      SyncEntry, Mutate, SyncDestination, ProcessDeath, WritebackEntry,
      WritebackLive, Restore, SealPrior, KeepForeign, DeleteEntry
<1>5. QED
  BY SMT, <1>1, <1>2, <1>3, <1>4

THEOREM ForeignObjectIsDurable ==
  ASSUME NEW c, CellInvariant(c, FALSE), Foreign(c)
  PROVE /\ c.cached.live = c.durable.live /\ ~UnsafeOwned(c, c.cached.live)
  BY SMT DEF CellInvariant, UndoEvidence, UnsafeOwned, Covered, Foreign

THEOREM ForeignRecoveryPreserved ==
  ASSUME NEW c, NEW next, CellInvariant(c, FALSE),
         c.control.stage = "recover", Foreign(c), CoreCellStep(c, next, FALSE)
  PROVE /\ next.cached.live = c.cached.live
        /\ next.durable.live = c.durable.live
  BY SMT, ForeignObjectIsDurable
  DEF CellInvariant, CellSpace, CoreCellStep, Foreign,
      WritePrior, SyncPrior, WriteEntry, SyncEntry, Mutate, SyncDestination,
      ProcessDeath, PowerLoss, WritebackEntry, WritebackLive,
      Restore, SealPrior, KeepForeign, DeleteEntry

THEOREM CellImagesRemainTyped ==
  ASSUME NEW cell \in CellSpace
  PROVE CellImages(cell) \subseteq CellSpace
<1> HIDE DEF PriorWriteImage, PriorSyncImage, EntryWriteImage, EntrySyncImage,
             MutationImage, DestinationSyncImage, ProcessDeathImage, EntryWritebackImage,
             LiveWritebackImage, RestorationImage, PriorSealImage, KeepImage, EntryRemovalImage,
             ExternalEditImage, PowerLossImage
<1>1. {PriorWriteImage(cell), PriorSyncImage(cell), KeepImage(cell)} \subseteq CellSpace
  BY SMT DEF PriorWriteImage, PriorSyncImage, KeepImage, CellSpace, DiskSpace, ControlSpace
<1>2. {EntrySyncImage(cell), DestinationSyncImage(cell), ProcessDeathImage(cell),
        EntryWritebackImage(cell), LiveWritebackImage(cell)} \subseteq CellSpace
  BY SMT
  DEF EntrySyncImage, DestinationSyncImage, ProcessDeathImage, EntryWritebackImage,
      LiveWritebackImage, CellSpace, DiskSpace, ControlSpace, Stages
<1>3. {MutationImage(cell), RestorationImage(cell), PriorSealImage(cell), EntryRemovalImage(cell)} \subseteq CellSpace
  BY SMT, LiveType, MissingType, CellValueTypes
  DEF MutationImage, RestorationImage, PriorSealImage, EntryRemovalImage, CellSpace, DiskSpace,
      ControlSpace, Stages, Owners
<1>4. \A value \in Objects : EntryWriteImage(cell, value) \in CellSpace
  BY SMT, PresentEntryType, CellValueTypes
  DEF EntryWriteImage, CellSpace, DiskSpace, ControlSpace, Stages
<1>5. \A value \in Objects : ExternalEditImage(cell, value) \in CellSpace
  BY SMT, LiveType DEF ExternalEditImage, CellSpace, DiskSpace, Owners
<1>6. PowerLossImages(cell) \subseteq CellSpace
  BY SMT DEF PowerLossImages, PowerLossImage, CellSpace, DiskSpace, ControlSpace, Stages
<1>7. QED
  BY SMT, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6 DEF CellImages

THEOREM PointwiseCellPreservation ==
  ASSUME NEW Domain, NEW before \in [Domain -> CellSpace], NEW after \in [Domain -> CellSpace],
         NEW wasCommitted \in BOOLEAN, NEW isCommitted \in BOOLEAN,
         wasCommitted => isCommitted,
         \A destination \in Domain : CellInvariant(before[destination], wasCommitted),
         \A destination \in Domain :
           after[destination] = before[destination] \/
           CellStep(before[destination], after[destination], wasCommitted)
  PROVE \A destination \in Domain : CellInvariant(after[destination], isCommitted)
  BY SMT, CellStepSafety, CommitMonotonicity

=============================================================================
