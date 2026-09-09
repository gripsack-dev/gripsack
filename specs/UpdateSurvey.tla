----------------------------- MODULE UpdateSurvey -----------------------------
(***************************************************************************
0044 survey contract. report::update_model and CLI flows supply real-code bridges.
Each module preparation is assumed to terminate with a classified outcome.
Explore every order/outcome for a finite selected set, including no-source
modules. Setup/admission failures and cancellation precede or abort this
protocol; they cannot be reported as a completed/current survey.
***************************************************************************)
EXTENDS Naturals, FiniteSets
CONSTANTS Modules, StopOnFailure, ConflateFailureExit, PublishDuringSurvey
VARIABLES pending, results, finished, exitCode, lockPublished, cachePublished
variables == <<pending, results, finished, exitCode, lockPublished, cachePublished>>
Outcomes == {"current", "changed", "failed", "not-applicable"}
Has(outcome) == \E module \in Modules : results[module] = outcome
ExpectedExit == IF Has("failed") THEN 2 ELSE IF Has("changed") THEN 1 ELSE 0

Init ==
    /\ pending = Modules
    /\ results = [module \in Modules |-> "pending"]
    /\ finished = FALSE
    /\ exitCode = 3
    /\ lockPublished = FALSE
    /\ cachePublished = FALSE

CompleteModule(module, outcome) ==
    /\ ~finished /\ module \in pending /\ outcome \in Outcomes
    /\ pending' = pending \ {module}
    /\ results' = [results EXCEPT ![module] = outcome]
    /\ finished' = (StopOnFailure /\ outcome = "failed")
    /\ exitCode' = IF finished' THEN 2 ELSE exitCode
    /\ lockPublished' = lockPublished
    /\ cachePublished' = (cachePublished \/ (PublishDuringSurvey /\ outcome = "changed"))

Finish ==
    /\ ~finished /\ pending = {}
    /\ finished' = TRUE
    /\ exitCode' = IF ConflateFailureExit /\ Has("failed") THEN 1 ELSE ExpectedExit
    /\ UNCHANGED <<pending, results, lockPublished, cachePublished>>

Next == (\E module \in Modules, outcome \in Outcomes : CompleteModule(module, outcome)) \/ Finish
Spec == Init /\ [][Next]_variables /\ WF_variables(Next)

TypeOK ==
    /\ pending \subseteq Modules
    /\ results \in [Modules -> Outcomes \union {"pending"}]
    /\ finished \in BOOLEAN /\ exitCode \in 0..3
    /\ lockPublished \in BOOLEAN /\ cachePublished \in BOOLEAN
EachModuleAccounted == \A module \in Modules : (module \in pending) = (results[module] = "pending")
CompleteSurvey == finished => pending = {}
HonestExit == finished => exitCode = ExpectedExit
NoPublication == ~lockPublished /\ ~cachePublished
EventuallyFinishes == <>finished
=============================================================================
