---- MODULE UndoCell ----
EXTENDS Integers, FiniteSets

CONSTANTS Objects, Absent
ASSUME ObjectDomain == Absent \in Objects

Owners == {"initial", "core", "external"}
Stages == {"capture", "entryDirty", "entryDurable", "destinationDirty", "idle", "recover"}
Live(value, owner) == [value |-> value, owner |-> owner]
Entry(before, after) == [present |-> TRUE, before |-> before, after |-> after]
Missing(original) == [present |-> FALSE, before |-> original, after |-> original]
LiveSpace == [value: Objects, owner: Owners]
EntrySpace == [present: BOOLEAN, before: Objects, after: Objects]
DiskSpace == [live: LiveSpace, entry: EntrySpace, prior: BOOLEAN]
ControlSpace == [stage: Stages, processed: BOOLEAN]
CellSpace == [original: Objects, cached: DiskSpace, durable: DiskSpace, control: ControlSpace]

InitialDisk(original) == [live |-> Live(original, "initial"),
                          entry |-> Missing(original), prior |-> FALSE]
\* Admission observes a cached original; an earlier external write need not
\* already be stable. The prior snapshot, not an invented source-file fsync,
\* supplies restoration authority before this run can mutate the destination.
InitialCell(original, stableOriginal) ==
    [original |-> original, cached |-> InitialDisk(original),
     durable |-> [InitialDisk(original) EXCEPT !.live = Live(stableOriginal, "initial")],
     control |-> [stage |-> "capture", processed |-> FALSE]]
InitialCells ==
    {InitialCell(pair[1], pair[2]) : pair \in Objects \X Objects}

UnsafeOwned(c, live) == live.owner = "core" /\ live.value # c.original
Covered(entry, live) == entry.present /\ live.value \in {entry.before, entry.after}
Foreign(c) == c.cached.live.value \notin
    {c.original, Absent, c.cached.entry.before, c.cached.entry.after}

\* Four cross-layer relations admit independent writeback of each field.
\* In particular a v2 update retains the original prior and records the
\* immediately preceding owned value; an old destination and new entry
\* must remain recoverable together.
UndoEvidence(c, committed) ==
    ~committed =>
      /\ (UnsafeOwned(c, c.cached.live) =>
            /\ Covered(c.cached.entry, c.cached.live)
            /\ Covered(c.durable.entry, c.cached.live))
      /\ (UnsafeOwned(c, c.durable.live) =>
            /\ Covered(c.cached.entry, c.durable.live)
            /\ Covered(c.durable.entry, c.durable.live))

WriterState(c) ==
    /\ (c.control.stage = "capture" =>
          /\ ~c.cached.entry.present /\ ~c.durable.entry.present
          /\ c.cached.live = Live(c.original, "initial")
          /\ c.durable.live.owner = "initial")
    /\ (c.control.stage = "entryDirty" =>
          /\ c.cached.entry.present
          /\ (c.cached.live = c.durable.live \/
                (c.cached.live = Live(c.original, "initial") /\ c.durable.live.owner = "initial"))
          /\ c.cached.live.value = c.cached.entry.before)
    /\ (c.control.stage = "entryDurable" =>
          /\ c.cached.entry.present /\ c.durable.entry = c.cached.entry
          /\ (c.cached.live = c.durable.live \/
                (c.cached.live = Live(c.original, "initial") /\ c.durable.live.owner = "initial"))
          /\ c.cached.live.value = c.cached.entry.before)
    /\ (c.control.stage = "destinationDirty" =>
          /\ c.cached.entry.present /\ c.durable.entry = c.cached.entry
          /\ c.cached.live = Live(c.cached.entry.after, "core")
          /\ (c.durable.live.owner = "initial" \/
                c.durable.live.value \in {c.cached.entry.before, c.cached.entry.after}))
    /\ (c.control.stage = "idle" =>
          /\ c.cached.entry.present /\ c.durable.entry = c.cached.entry
          /\ c.cached.live = c.durable.live)
    /\ (c.control.stage # "recover" => ~c.control.processed)

CellState(c, committed) ==
    /\ c.durable.prior => c.cached.prior
    /\ (c.cached.entry.present \/ c.durable.entry.present) => c.durable.prior
    /\ (c.cached.live.owner # "core" /\ c.cached.live.value # c.original) =>
         c.cached.live = c.durable.live
    /\ c.control.processed =>
         /\ c.cached.live = c.durable.live
         /\ ~UnsafeOwned(c, c.cached.live)
    /\ WriterState(c)
    /\ UndoEvidence(c, committed)

CellInvariant(c, committed) == c \in CellSpace /\ CellState(c, committed)

\* Each image has one definition, shared by the relational proof rules and
\* TLC's finite successor enumeration. No second hand-copied executor.
PriorWriteImage(c) == [c EXCEPT !.cached.prior = TRUE]
PriorSyncImage(c) == [c EXCEPT !.durable.prior = TRUE]
EntryWriteImage(c, value) ==
    [c EXCEPT !.cached.entry = Entry(c.cached.live.value, value),
              !.control.stage = "entryDirty"]
EntrySyncImage(c) ==
    [c EXCEPT !.durable.entry = c.cached.entry, !.control.stage = "entryDurable"]
MutationImage(c) ==
    [c EXCEPT !.cached.live = Live(c.cached.entry.after, "core"),
              !.control.stage = "destinationDirty"]
DestinationSyncImage(c) ==
    [c EXCEPT !.durable.live = c.cached.live, !.control.stage = "idle"]
ExternalEditImage(c, value) ==
    [c EXCEPT !.cached.live = Live(value, "external"),
              !.durable.live = Live(value, "external")]
ProcessDeathImage(c) ==
    [c EXCEPT !.control.stage = "recover", !.control.processed = FALSE]
EntryWritebackImage(c) == [c EXCEPT !.durable.entry = c.cached.entry]
LiveWritebackImage(c) == [c EXCEPT !.durable.live = c.cached.live]
PowerLossImage(c, entry, live, prior) ==
    [c EXCEPT !.cached.entry = entry, !.durable.entry = entry,
              !.cached.live = live, !.durable.live = live,
              !.cached.prior = prior, !.durable.prior = prior,
              !.control.stage = "recover", !.control.processed = FALSE]
RestorationImage(c) ==
    [c EXCEPT !.cached.live = Live(c.original, "core"), !.control.processed = FALSE]
PriorSealImage(c) ==
    [c EXCEPT !.durable.live = c.cached.live, !.control.processed = TRUE]
KeepImage(c) == [c EXCEPT !.control.processed = TRUE]
EntryRemovalImage(c) == [c EXCEPT !.cached.entry = Missing(c.original)]

WritePrior(c, next) ==
    /\ c.control.stage = "capture"
    /\ next = PriorWriteImage(c)
SyncPrior(c, next) ==
    /\ c.cached.prior
    /\ next = PriorSyncImage(c)
WriteEntry(c, next, value) ==
    /\ c.control.stage \in {"capture", "idle"}
    /\ c.durable.prior
    /\ c.control.stage = "idle" => c.cached.live.value = c.cached.entry.after
    /\ next = EntryWriteImage(c, value)
SyncEntry(c, next) ==
    /\ c.control.stage = "entryDirty"
    /\ next = EntrySyncImage(c)
Mutate(c, next) ==
    /\ c.control.stage = "entryDurable"
    /\ next = MutationImage(c)
SyncDestination(c, next) ==
    /\ c.control.stage = "destinationDirty"
    /\ next = DestinationSyncImage(c)

\* A user-published, durable foreign object may arrive between core
\* operations, including between recovery attempts. The observer/effect
\* critical section has the same no-portable-CAS assumption as production.
ExternalEdit(c, next, value) ==
    /\ c.control.stage \in {"idle", "recover"}
    /\ next = ExternalEditImage(c, value)
ProcessDeath(c, next) ==
    next = ProcessDeathImage(c)

\* Writeback is independent of successful acknowledgement by the caller.
\* Fsync orders these individual fields; it is not one opaque durable-write
\* action. Power loss independently chooses a permitted old/new version.
WritebackEntry(c, next) == next = EntryWritebackImage(c)
WritebackLive(c, next) == next = LiveWritebackImage(c)
PowerLoss(c, next) ==
    \E entry \in {c.cached.entry, c.durable.entry},
       live \in {c.cached.live, c.durable.live},
       prior \in {c.cached.prior, c.durable.prior} :
      next = PowerLossImage(c, entry, live, prior)

Restore(c, next) ==
    /\ c.control.stage = "recover" /\ c.cached.entry.present
    /\ c.cached.live.value # c.original /\ ~Foreign(c)
    /\ next = RestorationImage(c)
SealPrior(c, next) ==
    /\ c.control.stage = "recover" /\ c.cached.entry.present
    /\ c.cached.live.value = c.original
    /\ next = PriorSealImage(c)
KeepForeign(c, next) ==
    /\ c.control.stage = "recover" /\ c.cached.entry.present /\ Foreign(c)
    /\ next = KeepImage(c)
DeleteEntry(c, next, committed) ==
    /\ c.control.stage = "recover" /\ c.cached.entry.present
    /\ committed \/ c.control.processed
    /\ next = EntryRemovalImage(c)

CoreCellStep(c, next, committed) ==
    \/ WritePrior(c, next) \/ SyncPrior(c, next)
    \/ \E value \in Objects : WriteEntry(c, next, value)
    \/ SyncEntry(c, next) \/ Mutate(c, next) \/ SyncDestination(c, next)
    \/ ProcessDeath(c, next) \/ PowerLoss(c, next)
    \/ WritebackEntry(c, next) \/ WritebackLive(c, next)
    \/ (~committed /\ (Restore(c, next) \/ SealPrior(c, next) \/ KeepForeign(c, next)))
    \/ DeleteEntry(c, next, committed)

CellStep(c, next, committed) ==
    \/ CoreCellStep(c, next, committed)
    \/ \E value \in Objects : ExternalEdit(c, next, value)

PowerLossImages(c) ==
    {PowerLossImage(c, choice[1], choice[2], choice[3]) :
        choice \in {c.cached.entry, c.durable.entry}
                \X {c.cached.live, c.durable.live}
                \X {c.cached.prior, c.durable.prior}}
CellImages(c) ==
    {PriorWriteImage(c), PriorSyncImage(c), EntrySyncImage(c), MutationImage(c),
     DestinationSyncImage(c), ProcessDeathImage(c), EntryWritebackImage(c),
     LiveWritebackImage(c), RestorationImage(c), PriorSealImage(c),
     KeepImage(c), EntryRemovalImage(c)}
    \union {EntryWriteImage(c, value) : value \in Objects}
    \union {ExternalEditImage(c, value) : value \in Objects}
    \union PowerLossImages(c)
CellSuccessors(c, committed) ==
    {next \in CellImages(c) : CellStep(c, next, committed)}

=============================================================================
