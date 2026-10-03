-------------------- MODULE BuildSessionFenceWitness --------------------
EXTENDS BuildSession, TLAPS

\* Negative calibration only. No induction theorem is imported. TLC supplies
\* reachable instances of the same unsafe switches after these step witnesses.
CONSTANT BadWorker
ASSUME WitnessDomain ==
    /\ CurrentSession \in Sessions /\ CurrentAttempt \in Attempts /\ CurrentEpoch \in Epochs
    /\ CurrentWorker \in Workers /\ BadWorker \in Workers /\ BadWorker # CurrentWorker
    /\ CurrentWorkerEpoch \in WorkerEpochs
BadKey == <<CurrentSession, CurrentAttempt, CurrentEpoch, BadWorker, CurrentWorkerEpoch>>
WorkerSubstitution ==
    /\ Init
    /\ stage' = "running" /\ exportKey' = NONE /\ process' = "running"
    /\ protocolFailed' = FALSE /\ authorized' = FALSE
    /\ seen' = {"accepted"} /\ failedPayload' = NO_FAILURE /\ replayed' = FALSE
    /\ foreignSeen' = TRUE /\ conflictSeen' = FALSE

THEOREM WorkerWitnessStartsSafe == Init => Inv
    BY SMT, WitnessDomain
    DEF Init, Inv, TypeOK, MatchingExport, ForeignRefusal, ConflictRefusal,
        CompleteOnly, Stages, Events, Payloads, Keys, Expected

THEOREM MissingWorkerFenceAcceptsForeignReply ==
    ASSUME IgnoreWorker, WorkerSubstitution
    PROVE Receive(BadKey, "accepted", "first") /\ ~ForeignRefusal'
    BY SMT, WitnessDomain
    DEF WorkerSubstitution, Init, Receive, IdentityMatches, Replay, ReplayReply,
        RejectReply, AdvanceReply, Allowed, Advance, TerminalMismatch,
        ForeignRefusal, BadKey, Expected, Terminals

FailedBefore ==
    /\ stage = "failed" /\ exportKey = NONE /\ process = "running"
    /\ protocolFailed = FALSE /\ authorized = FALSE
    /\ seen = {"failed"} /\ failedPayload = "first" /\ replayed = FALSE
    /\ foreignSeen = FALSE /\ conflictSeen = FALSE
ConflictingFailure ==
    /\ FailedBefore /\ replayed' = TRUE /\ conflictSeen' = TRUE
    /\ UNCHANGED <<stage, exportKey, process, protocolFailed, authorized,
                   seen, failedPayload, foreignSeen>>

THEOREM ConflictWitnessStartsSafe == FailedBefore => Inv
    BY SMT, WitnessDomain
    DEF FailedBefore, Inv, TypeOK, MatchingExport, ForeignRefusal,
        ConflictRefusal, CompleteOnly, Stages, Events, Payloads

THEOREM MissingConflictFenceAdmitsChangedTerminal ==
    ASSUME IgnoreConflict, ConflictingFailure
    PROVE Receive(Expected, "failed", "second") /\ ~ConflictRefusal'
    BY SMT
    DEF ConflictingFailure, FailedBefore, Receive, IdentityMatches,
        Replay, ReplayReply, RejectReply, AdvanceReply, Allowed, Advance,
        TerminalMismatch, ConflictRefusal, Terminals
=============================================================================
