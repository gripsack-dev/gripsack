------------------------- MODULE ProcessSupervision -------------------------
EXTENDS Naturals, TLC

(***************************************************************************
 A finite, single invocation of the shared mechanism, NOT a protocol-success
 oracle. Each I/O action is one nonblocking poll/read/write/callback step.
 EAGAIN is represented by taking another action (including Tick), never by
 blocking time. Input/output units and callback lines are abstract bytes.

 Tick is weakly fair, even after the bounded clock saturates: pulse keeps
 polling observable and prevents terminal/deadline deadlocks from masquerading
 as a liveness proof. Callback steps terminate; their time is charged by Tick.
 The active budget ends at Deadline - Reserve, not at Deadline.

 OSExit and OSKill are weakly fair. This is an explicit cooperative-kernel
 assumption, NOT a bound on kernel scheduling. Fairness need not schedule them
 inside Reserve: expiry must honestly report cleanup failure. The fault peer
 represents an explicit kill error. Retained authority on cleanup failure is
 intentional: this model never authorizes a later kill through a reused PID.
 No physical real-time or hostile-kernel guarantee is asserted.
***************************************************************************)
CONSTANTS Deadline, Reserve, InputLimit, OutputLimit, LineLimit, TailLimit,
          Mutant
ASSUME /\ Deadline \in Nat /\ Reserve \in 1..Deadline
       /\ Deadline > Reserve
       /\ InputLimit \in Nat /\ InputLimit > 0
       /\ OutputLimit \in Nat /\ OutputLimit > 0
       /\ LineLimit \in 1..OutputLimit
       /\ TailLimit \in 0..OutputLimit
       /\ Mutant \in {"none", "ignoreDeadline", "waitInherited", "reapFirst"}

Peers == {"normal", "blocked", "flood", "linger", "inherited", "fault"}
Reasons == {"none", "input", "response", "exit", "deadline", "output", "line"}
Min(a, b) == IF a < b THEN a ELSE b
VARIABLE s
vars == <<s>>

Init ==
  \E peer \in Peers, size \in {InputLimit, InputLimit + 1}:
    s = [phase |-> IF size > InputLimit THEN "done" ELSE "active",
         peer |-> peer, size |-> size, now |-> 0, pulse |-> FALSE,
         sent |-> 0, stdinClosed |-> size > InputLimit,
         out |-> 0, err |-> 0, line |-> 0, tail |-> 0,
         response |-> FALSE, leaderDead |-> size > InputLimit,
         pipesClosed |-> size > InputLimit,
         authority |-> size <= InputLimit, killSent |-> FALSE,
         reaped |-> FALSE, localClosed |-> size > InputLimit,
         reason |-> IF size > InputLimit THEN "input" ELSE "none",
         cleanup |-> IF size > InputLimit THEN "notSpawned" ELSE "pending"]

TypeOK ==
  s \in [phase : {"active", "cleanup", "done"}, peer : Peers,
         size : {InputLimit, InputLimit + 1}, now : 0..Deadline,
         pulse : BOOLEAN, sent : 0..InputLimit, stdinClosed : BOOLEAN,
         out : 0..OutputLimit, err : 0..OutputLimit,
         line : 0..LineLimit, tail : 0..TailLimit,
         response : BOOLEAN, leaderDead : BOOLEAN, pipesClosed : BOOLEAN,
         authority : BOOLEAN, killSent : BOOLEAN, reaped : BOOLEAN,
         localClosed : BOOLEAN, reason : Reasons,
         cleanup : {"pending", "ok", "failed", "notSpawned"}]

Tick == s' = [s EXCEPT !.now = Min(@ + 1, Deadline), !.pulse = ~@]
Active == s.phase = "active"
Stop(why) == s' = [s EXCEPT !.phase = "cleanup", !.reason = why]

Send == /\ Active /\ s.peer # "blocked" /\ s.sent < s.size
        /\ s' = [s EXCEPT !.sent = @ + 1,
                         !.stdinClosed = (s.sent + 1 = s.size)]

\* A peer cannot allocate beyond a cap: the next byte is rejected instead.
ReadOut ==
  /\ Active /\ ~s.leaderDead /\ s.peer # "blocked"
  /\ IF s.out = OutputLimit THEN Stop("output")
     ELSE IF s.line = LineLimit THEN Stop("line")
     ELSE s' = [s EXCEPT !.out = @ + 1, !.line = @ + 1]
ReadErr ==
  /\ Active /\ ~s.leaderDead /\ s.peer # "blocked"
  /\ IF s.err = OutputLimit THEN Stop("output")
     ELSE s' = [s EXCEPT !.err = @ + 1,
                         !.tail = Min(@ + 1, TailLimit)]
\* Complete bounded callback lines; parsed messages are not queued.
ContinueLine == /\ Active /\ s.line > 0
                /\ s' = [s EXCEPT !.line = 0]
Response ==
  /\ Active /\ s.stdinClosed /\ s.line > 0
  /\ s.peer \in {"normal", "linger", "fault"}
  /\ s' = [s EXCEPT !.response = TRUE, !.line = 0,
                   !.phase = "cleanup", !.reason = "response"]

OSExit ==
  /\ Active /\ s.stdinClosed /\ ~s.leaderDead
  /\ s.peer \in {"normal", "inherited"}
  /\ s' = [s EXCEPT !.leaderDead = TRUE,
                   !.pipesClosed = (s.peer # "inherited")]
ObserveExit == /\ Active /\ s.leaderDead /\ s.pipesClosed
               /\ Stop("exit")
Cutoff ==
  /\ Active /\ s.now >= Deadline - Reserve
  /\ Mutant # "ignoreDeadline"
  /\ ~(Mutant = "waitInherited" /\ s.peer = "inherited"
        /\ ~s.pipesClosed)
  /\ Stop("deadline")

\* Kill uses the still-owned process-group identity, even for a dead leader.
Kill == /\ s.phase = "cleanup" /\ s.authority /\ ~s.killSent
        /\ s' = [s EXCEPT !.killSent = TRUE]
OSKill ==
  /\ s.phase = "cleanup" /\ s.killSent /\ s.peer # "fault"
  /\ (~s.leaderDead \/ ~s.pipesClosed)
  /\ s' = [s EXCEPT !.leaderDead = TRUE, !.pipesClosed = TRUE]
Reap ==
  /\ s.phase = "cleanup" /\ s.leaderDead /\ ~s.reaped
  /\ (s.killSent \/ Mutant = "reapFirst")
  /\ s' = [s EXCEPT !.reaped = TRUE, !.authority = FALSE]
CloseLocal == /\ s.phase = "cleanup" /\ ~s.localClosed
              /\ s' = [s EXCEPT !.localClosed = TRUE, !.stdinClosed = TRUE]
Clean == s.reaped /\ s.pipesClosed /\ s.localClosed /\ s.killSent
Finish == /\ s.phase = "cleanup" /\ Clean
          /\ s' = [s EXCEPT !.phase = "done", !.cleanup = "ok"]
CleanupFailure ==
  /\ s.phase = "cleanup" /\ ~Clean
  /\ (s.now = Deadline \/ (s.peer = "fault" /\ s.killSent))
  /\ s' = [s EXCEPT !.phase = "done", !.cleanup = "failed",
                   !.localClosed = TRUE, !.stdinClosed = TRUE]

Service == Send \/ ReadOut \/ ReadErr \/ ContinueLine \/ Response
           \/ ObserveExit \/ Cutoff \/ Kill \/ Reap \/ CloseLocal
           \/ Finish \/ CleanupFailure
Next == Tick \/ Service \/ OSExit \/ OSKill
Spec == Init /\ [][Next]_vars
        /\ WF_vars(Tick) /\ WF_vars(Service)
        /\ WF_vars(OSExit) /\ WF_vars(OSKill)

BoundedBuffers == /\ s.sent <= InputLimit /\ s.sent <= s.size
                  /\ s.out <= OutputLimit /\ s.err <= OutputLimit
                  /\ s.line <= LineLimit /\ s.line <= s.out
                  /\ s.tail <= TailLimit /\ s.tail <= s.err
PidAuthority == /\ (s.reaped => s.killSent)
                /\ (s.authority = (s.size <= InputLimit /\ ~s.reaped))
HonestCleanup ==
  /\ (s.cleanup = "ok" => Clean)
  /\ (s.phase = "done" =>
       /\ s.localClosed /\ s.stdinClosed /\ s.reason # "none"
       /\ s.cleanup # "pending"
       /\ (~Clean /\ s.size <= InputLimit => s.cleanup = "failed"))
Terminates == <>(s.phase = "done")
ClockProgress == <>(s.now = Deadline)
\* Useful coverage obligations: TLC should REFUTE each in a separate run.
NeverInherited == ~(s.peer = "inherited" /\ s.leaderDead /\ ~s.pipesClosed)
NeverLinger == ~(s.peer = "linger" /\ s.response /\ ~s.leaderDead)
NeverCleanupFailure == s.cleanup # "failed"
=============================================================================
