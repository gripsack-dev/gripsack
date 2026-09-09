------------------------------- MODULE HttpRetry -------------------------------
(***************************************************************************
0044 retry contract. Abstract clock ticks represent a monotonic operation-wide
budget; they are not seconds. Only classified idempotent GET failures replay.
A response here includes body completion/verification classification. HTTP
parsing, spool byte accounting, TLS and OS scheduling have Rust/flow bridges;
this abstraction alone cannot prove their implementations correct.
***************************************************************************)
EXTENDS Naturals
CONSTANTS AttemptLimit, TimeLimit, SleepLimit,
          RetryTerminalFailure, ResetDeadline, IgnoreAttemptLimit
VARIABLES phase, attempts, clock, deadline, sleepUsed, lastFailure,
          previousFailure, throttleAdmissions, method
variables == <<phase, attempts, clock, deadline, sleepUsed, lastFailure,
               previousFailure, throttleAdmissions, method>>
Failures == {"transient", "rate-limited", "forbidden", "integrity"}
Retryable == lastFailure \in {"transient", "rate-limited"}
AttemptsRemain == attempts < AttemptLimit \/ (IgnoreAttemptLimit /\ attempts = AttemptLimit)

Init ==
    /\ phase = "ready" /\ attempts = 0 /\ clock = 0
    /\ deadline = TimeLimit /\ sleepUsed = 0
    /\ lastFailure = "none" /\ previousFailure = "none"
    /\ throttleAdmissions = 0 /\ method \in {"GET", "POST"}

Start ==
    /\ phase = "ready" /\ clock < deadline /\ AttemptsRemain
    /\ phase' = "request" /\ attempts' = attempts + 1
    /\ throttleAdmissions' = throttleAdmissions + 1
    /\ previousFailure' = lastFailure
    /\ UNCHANGED <<clock, deadline, sleepUsed, lastFailure, method>>

Response(outcome) ==
    /\ phase = "request" /\ outcome \in Failures \union {"success"}
    /\ phase' = IF outcome = "success" THEN "done" ELSE "decide"
    /\ lastFailure' = IF outcome = "success" THEN lastFailure ELSE outcome
    /\ UNCHANGED <<attempts, clock, deadline, sleepUsed, previousFailure, throttleAdmissions, method>>

ScheduleRetry(wait) ==
    /\ phase = "decide" /\ method = "GET"
    /\ (Retryable \/ RetryTerminalFailure) /\ AttemptsRemain
    /\ wait \in 1..(SleepLimit + 1)
    /\ sleepUsed + wait <= SleepLimit /\ clock + wait < deadline
    /\ clock' = clock + wait /\ sleepUsed' = sleepUsed + wait
    /\ deadline' = IF ResetDeadline THEN clock' + TimeLimit ELSE deadline
    /\ phase' = "ready"
    /\ UNCHANGED <<attempts, lastFailure, previousFailure, throttleAdmissions, method>>

Stop ==
    /\ phase = "decide"
    /\ phase' = "done"
    /\ UNCHANGED <<attempts, clock, deadline, sleepUsed, lastFailure,
                   previousFailure, throttleAdmissions, method>>

Tick ==
    /\ phase # "done" /\ clock < deadline
    /\ clock' = clock + 1
    /\ UNCHANGED <<phase, attempts, deadline, sleepUsed, lastFailure,
                   previousFailure, throttleAdmissions, method>>

Timeout ==
    /\ phase # "done" /\ clock >= deadline
    /\ phase' = "done"
    /\ UNCHANGED <<attempts, clock, deadline, sleepUsed, lastFailure,
                   previousFailure, throttleAdmissions, method>>

Next == Start \/ (\E outcome \in Failures \union {"success"} : Response(outcome))
        \/ (\E wait \in 1..(SleepLimit + 1) : ScheduleRetry(wait)) \/ Stop \/ Tick \/ Timeout
Spec == Init /\ [][Next]_variables /\ WF_variables(Tick) /\ WF_variables(Timeout)

TypeOK ==
    /\ phase \in {"ready", "request", "decide", "done"}
    /\ attempts \in 0..(AttemptLimit + 1)
    /\ clock \in 0..((AttemptLimit + 1) * TimeLimit)
    /\ sleepUsed \in 0..SleepLimit
    /\ method \in {"GET", "POST"}
BoundedAttempts == attempts <= AttemptLimit
FixedDeadline == deadline = TimeLimit
EveryAttemptIsThrottled == throttleAdmissions = attempts
NoTerminalReplay == phase = "request" /\ attempts > 1
    => method = "GET" /\ previousFailure \in {"transient", "rate-limited"}
WithinBudget == clock <= deadline /\ sleepUsed <= SleepLimit
EventuallyStops == <>(phase = "done")
=============================================================================
