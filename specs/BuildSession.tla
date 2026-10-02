--------------------------- MODULE BuildSession ---------------------------
(***************************************************************************
The native bridge admission boundary, not the BuildKit solver. Completion
requires matching session/attempt/cancellation fence AND worker instance/epoch,
export, successful native process cleanup, and no protocol failure. Exact
control replays stutter; conflicting terminals are refused. Payload symbols
represent admitted capability receipts or complete failure (code/message/vertex)
receipts, never recipe content or executable authority.

Production: FromBridge::matches/worker_binding, buildkit::transition and
Bridge::invoke/execute. Parsing, hashing, OS cleanup and worker effects remain
separate tested/trusted boundaries. Publication follows a separate protocol.
***************************************************************************)
EXTENDS Naturals, FiniteSets
CONSTANTS Sessions, Attempts, Epochs, Workers, WorkerEpochs,
          CurrentSession, CurrentAttempt, CurrentEpoch, CurrentWorker, CurrentWorkerEpoch,
          IgnoreIdentity, IgnoreWorker, IgnoreConflict, DoneBeforeExport
Expected == <<CurrentSession, CurrentAttempt, CurrentEpoch, CurrentWorker, CurrentWorkerEpoch>>
Keys == Sessions \X Attempts \X Epochs \X Workers \X WorkerEpochs
NONE == <<"no-session", 0, 0, "no-worker", 0>>
NO_FAILURE == "no-failure"
Payloads == {"first", "second"}
Stages == {"awaiting", "running", "exported", "done", "failed", "cancelled"}
Events == {"accepted", "log", "exported", "done", "failed", "cancelled"}
Terminals == {"done", "failed", "cancelled"}
VARIABLES stage, exportKey, process, protocolFailed, authorized,
          seen, failedPayload, replayed, foreignSeen, conflictSeen
vars == <<stage, exportKey, process, protocolFailed, authorized,
          seen, failedPayload, replayed, foreignSeen, conflictSeen>>

IdentityMatches(key) == key = Expected \/ IgnoreIdentity
    \/ (IgnoreWorker /\ <<key[1], key[2], key[3]>> = <<CurrentSession, CurrentAttempt, CurrentEpoch>>)
Allowed(s, event) ==
    \/ /\ s = "awaiting" /\ event = "accepted"
    \/ /\ s = "running" /\ event \in {"log", "exported"}
    \/ /\ s = "exported" /\ event = "done"
    \/ /\ DoneBeforeExport /\ s = "running" /\ event = "done"
    \/ /\ s \notin Terminals /\ event \in {"failed", "cancelled"}
Advance(event) == CASE event = "accepted" -> "running"
                     [] event = "log" -> "running"
                     [] OTHER -> event
TerminalMismatch(event, payload) ==
    /\ stage \in Terminals /\ event \in Terminals
    /\ event # stage \/ (event = "failed" /\ payload # failedPayload)
Replay(event, payload) ==
    \/ /\ event \in {"accepted", "exported"} /\ event \in seen
    \/ /\ stage \in {"done", "cancelled"} /\ event = stage
    \/ /\ stage = "failed" /\ event = "failed" /\ payload = failedPayload
    \/ /\ IgnoreConflict /\ stage \in Terminals /\ event \in Terminals
RejectReply ==
    /\ protocolFailed' = TRUE
    /\ UNCHANGED <<stage, exportKey, seen, failedPayload, replayed>>
ReplayReply ==
    /\ replayed' = TRUE
    /\ UNCHANGED <<stage, exportKey, seen, failedPayload, protocolFailed>>
AdvanceReply(key, event, payload) ==
    /\ stage' = Advance(event)
    /\ exportKey' = IF event = "exported" THEN key ELSE exportKey
    /\ seen' = seen \cup {event}
    /\ failedPayload' = IF event = "failed" THEN payload ELSE failedPayload
    /\ UNCHANGED <<protocolFailed, replayed>>
Receive(key, event, payload) ==
    /\ process = "running" /\ ~protocolFailed
    /\ foreignSeen' = (foreignSeen \/ key # Expected)
    /\ conflictSeen' = (conflictSeen \/ TerminalMismatch(event, payload))
    /\ IF IdentityMatches(key) /\ (event # "accepted" \/ payload = "first")
       THEN IF Replay(event, payload) THEN ReplayReply
            ELSE IF Allowed(stage, event) THEN AdvanceReply(key, event, payload)
                 ELSE RejectReply
       ELSE RejectReply
    /\ UNCHANGED <<process, authorized>>

Exit(result) ==
    /\ process = "running" /\ process' = result
    /\ UNCHANGED <<stage, exportKey, protocolFailed, authorized,
                   seen, failedPayload, replayed, foreignSeen, conflictSeen>>
Complete ==
    /\ process = "success" /\ ~protocolFailed /\ stage = "done"
    /\ authorized' = TRUE
    /\ UNCHANGED <<stage, exportKey, process, protocolFailed,
                   seen, failedPayload, replayed, foreignSeen, conflictSeen>>
Init == /\ stage = "awaiting" /\ exportKey = NONE
        /\ process = "running" /\ protocolFailed = FALSE /\ authorized = FALSE
        /\ seen = {} /\ failedPayload = NO_FAILURE /\ replayed = FALSE
        /\ foreignSeen = FALSE /\ conflictSeen = FALSE
Next == \/ \E key \in Keys, event \in Events, payload \in Payloads : Receive(key, event, payload)
        \/ \E result \in {"success", "failure", "unknown"} : Exit(result)
        \/ Complete
Spec == Init /\ [][Next]_vars

TypeOK == /\ stage \in Stages /\ exportKey \in Keys \cup {NONE}
          /\ process \in {"running", "success", "failure", "unknown"}
          /\ protocolFailed \in BOOLEAN /\ authorized \in BOOLEAN
          /\ seen \subseteq Events /\ failedPayload \in Payloads \cup {NO_FAILURE}
          /\ replayed \in BOOLEAN /\ foreignSeen \in BOOLEAN /\ conflictSeen \in BOOLEAN
MatchingExport == stage \in {"exported", "done"} => exportKey = Expected
ForeignRefusal == foreignSeen => protocolFailed
ConflictRefusal == conflictSeen => protocolFailed
CompleteOnly == authorized => /\ stage = "done" /\ process = "success"
                            /\ ~protocolFailed /\ exportKey = Expected
                            /\ ~foreignSeen /\ ~conflictSeen
Inv == TypeOK /\ MatchingExport /\ ForeignRefusal /\ ConflictRefusal /\ CompleteOnly
NeverCompletes == ~authorized
NeverCancellation == stage # "cancelled"
NeverReplay == ~replayed
===========================================================================
