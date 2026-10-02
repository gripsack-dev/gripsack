------------------------ MODULE BuildSessionProofs ------------------------
EXTENDS BuildSession, TLAPS

ASSUME AdmittedParameters ==
    /\ CurrentSession \in Sessions /\ CurrentAttempt \in Attempts /\ CurrentEpoch \in Epochs
    /\ CurrentWorker \in Workers /\ CurrentWorkerEpoch \in WorkerEpochs
    /\ NONE \notin Keys /\ ~IgnoreIdentity /\ ~IgnoreWorker /\ ~IgnoreConflict /\ ~DoneBeforeExport

THEOREM InitInvariant == Init => Inv
    BY SMT, AdmittedParameters
    DEF Init, Inv, TypeOK, MatchingExport, ForeignRefusal, ConflictRefusal, CompleteOnly,
        Stages, Events, Payloads, Keys, Expected

THEOREM ReceiveInvariant ==
    ASSUME NEW key \in Keys, NEW event \in Events, NEW payload \in Payloads, Inv, Receive(key, event, payload)
    PROVE Inv'
    BY SMT, AdmittedParameters
    DEF Inv, TypeOK, MatchingExport, ForeignRefusal, ConflictRefusal, CompleteOnly,
        Receive, IdentityMatches, Allowed, Advance, TerminalMismatch, Replay,
        RejectReply, ReplayReply, AdvanceReply, Stages, Events, Payloads, Terminals

THEOREM ExitInvariant ==
    ASSUME NEW result \in {"success", "failure", "unknown"}, Inv, Exit(result)
    PROVE Inv'
    BY SMT
    DEF Inv, TypeOK, MatchingExport, ForeignRefusal, ConflictRefusal, CompleteOnly, Exit

THEOREM CompletionInvariant ==
    ASSUME Inv, Complete
    PROVE Inv'
    BY SMT
    DEF Inv, TypeOK, MatchingExport, ForeignRefusal, ConflictRefusal, CompleteOnly, Complete

THEOREM StepInvariant == Inv /\ Next => Inv'
    BY SMT, ReceiveInvariant, ExitInvariant, CompletionInvariant DEF Next

THEOREM ExportAndProcessBeforeAuthority ==
    Inv /\ authorized => /\ exportKey = Expected /\ stage = "done"
                         /\ process = "success" /\ ~protocolFailed
    BY DEF Inv, CompleteOnly
===========================================================================
