---- MODULE RepeatedActivation ----
(***************************************************************************
 Conditional finite-crash completion of the CURRENT activation protocol.
 This wrapper reuses Activation's actual transition relation; it is not a
 second generation-only recovery engine. The explicit cfg crash budget is a
 liveness-exploration assumption, never a production retry/crash limit.
 MaxAttempts must exceed CrashBudget so a final uninterrupted attempt remains.
 Weak fairness is required, and native return is abstracted as an enabled
 transition. OS scheduling, arbitrary remote success and generalized M-V6
 safety are separate obligations.
***************************************************************************)
EXTENDS Activation
CONSTANT CrashBudget
ASSUME CrashBudget \in Nat /\ MaxAttempts > CrashBudget
VARIABLE crashes
completionVars == <<vars, crashes>>
CompletionInit == Init /\ crashes = 0
FairProgress == Progress /\ UNCHANGED crashes
BoundedCrash == (ProcessDeath \/ PowerLoss) /\ crashes < CrashBudget /\ crashes' = crashes + 1
CompletionNext == FairProgress \/ BoundedCrash
FairSpec == CompletionInit /\ [][CompletionNext]_completionVars /\ WF_completionVars(FairProgress)
CrashBound == crashes \in 0..CrashBudget
RecoveryCompletes == <>[](phase = "idle" /\ pendingC = NONE /\ pendingD = NONE)
=============================================================================
