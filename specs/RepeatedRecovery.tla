---- MODULE RepeatedRecovery ----
\* Conditional finite-work completion explorer. The safety model has no crash
\* count. Only this liveness wrapper assumes a finite crash/edit budget,
\* successful scheduled I/O, one requested write per destination, admitted
\* generations and no foreign writer of private control metadata.
EXTENDS JournalLifecycle

CONSTANTS PreviousGeneration, TargetGeneration, TargetTransaction, WorkKind,
          PriorValue, DeployedValue, EditedValue, CrashBudget
ASSUME CompletionDomain ==
    /\ PreviousGeneration \in GenerationIds /\ TargetGeneration \in AvailableGenerations
    /\ TargetTransaction \in Transactions \ Selection!InitialTransactions
    /\ WorkKind \in {"deploy", "prune"} /\ CrashBudget \in Nat
    /\ {PriorValue, DeployedValue, EditedValue} \subseteq Objects
    /\ Cardinality({PriorValue, DeployedValue, EditedValue, Absent}) = 4
PriorSelection == <<Legacy, PreviousGeneration>>
InitialContent == IF WorkKind = "deploy" THEN PriorValue ELSE DeployedValue
RequestedContent == IF WorkKind = "deploy" THEN DeployedValue ELSE Absent
TargetSelection == <<TargetTransaction, TargetGeneration>>

VARIABLES started, crashes, edited
completionVars == <<vars, started, crashes, edited>>
CompletionInit == Init /\ cells = [destination \in Destinations |-> InitialCell(InitialContent, InitialContent)]
    /\ started = FALSE /\ crashes = 0 /\ edited = {}
BeginFixedRun == ~started /\ BeginEpoch(TargetTransaction, TargetGeneration) /\ started' = TRUE
FiniteWriterStep(destination) ==
    /\ cells[destination].control.stage # "idle" /\ WriterStep(destination)
    /\ (cells'[destination].cached.entry.present => cells'[destination].cached.entry.after = RequestedContent)
FinishFixedWrites ==
    /\ (\A destination \in Destinations : cells[destination].control.stage = "idle") /\ FinishWriting
FiniteProgress ==
    /\ (BeginFixedRun \/
          (/\ started
           /\ ((mode \notin {"idle", "writing"} /\ CoreNext(AvailableGenerations))
               \/ (\E destination \in Destinations : FiniteWriterStep(destination)) \/ FinishFixedWrites
               \/ WritebackSelection \/ (\E destination \in Destinations : WritebackDestination(destination)))
           /\ UNCHANGED started))
    /\ UNCHANGED <<crashes, edited>>
BoundedCrash ==
    /\ started /\ crashes < CrashBudget /\ (CrashProcess \/ CrashPower)
    /\ crashes' = crashes + 1 /\ UNCHANGED <<started, edited>>
BoundedEdit(destination) ==
    /\ started /\ mode # "idle" /\ destination \notin edited
    /\ ExternalDestination(destination, EditedValue)
    /\ edited' = edited \union {destination} /\ UNCHANGED <<started, crashes>>
CompletionNext == FiniteProgress \/ BoundedCrash \/ (\E destination \in Destinations : BoundedEdit(destination))
FairSpec == CompletionInit /\ [][CompletionNext]_completionVars /\ WF_completionVars(FiniteProgress)

CompletionTypes == started \in BOOLEAN /\ crashes \in 0..CrashBudget /\ edited \subseteq Destinations
Recovered == started /\ mode = "idle" /\ CachedEntriesEmpty /\ StableEntriesEmpty
    /\ ~markerVisible.present /\ ~markerStable.present
PreservedEdits == \A destination \in edited : cells[destination].durable.live.value = EditedValue
RecoveryOracle == Recovered => \A destination \in Destinations : cells[destination].durable.live.value =
    IF destination \in edited THEN EditedValue
    ELSE IF currentStable = TargetSelection THEN RequestedContent ELSE InitialContent
CleanRunCommits == Recovered /\ crashes = 0 => currentStable = TargetSelection
RecoveryCompletes == <>[]Recovered

=============================================================================
