---- MODULE ActivationMutant_TTrace_1788632505 ----
EXTENDS Sequences, TLCExt, Toolbox, Naturals, TLC, ActivationMutant

_expression ==
    LET ActivationMutant_TEExpression == INSTANCE ActivationMutant_TEExpression
    IN ActivationMutant_TEExpression!expression
----

_trace ==
    LET ActivationMutant_TETrace == INSTANCE ActivationMutant_TETrace
    IN ActivationMutant_TETrace!trace
----

_inv ==
    ~(
        TLCGet("level") = Len(_TETrace)
        /\
        phase = ("idle")
        /\
        current = (1)
        /\
        pending = (NONE)
        /\
        activated = ({})
    )
----

_init ==
    /\ pending = _TETrace[1].pending
    /\ current = _TETrace[1].current
    /\ phase = _TETrace[1].phase
    /\ activated = _TETrace[1].activated
----

_next ==
    /\ \E i,j \in DOMAIN _TETrace:
        /\ \/ /\ j = i + 1
              /\ i = TLCGet("level")
        /\ pending  = _TETrace[i].pending
        /\ pending' = _TETrace[j].pending
        /\ current  = _TETrace[i].current
        /\ current' = _TETrace[j].current
        /\ phase  = _TETrace[i].phase
        /\ phase' = _TETrace[j].phase
        /\ activated  = _TETrace[i].activated
        /\ activated' = _TETrace[j].activated

\* Uncomment the ASSUME below to write the states of the error trace
\* to the given file in Json format. Note that you can pass any tuple
\* to `JsonSerialize`. For example, a sub-sequence of _TETrace.
    \* ASSUME
    \*     LET J == INSTANCE Json
    \*         IN J!JsonSerialize("ActivationMutant_TTrace_1788632505.json", _TETrace)

=============================================================================

 Note that you can extract this module `ActivationMutant_TEExpression`
  to a dedicated file to reuse `expression` (the module in the 
  dedicated `ActivationMutant_TEExpression.tla` file takes precedence 
  over the module `ActivationMutant_TEExpression` below).

---- MODULE ActivationMutant_TEExpression ----
EXTENDS Sequences, TLCExt, Toolbox, Naturals, TLC, ActivationMutant

expression == 
    [
        \* To hide variables of the `ActivationMutant` spec from the error trace,
        \* remove the variables below.  The trace will be written in the order
        \* of the fields of this record.
        pending |-> pending
        ,current |-> current
        ,phase |-> phase
        ,activated |-> activated
        
        \* Put additional constant-, state-, and action-level expressions here:
        \* ,_stateNumber |-> _TEPosition
        \* ,_pendingUnchanged |-> pending = pending'
        
        \* Format the `pending` variable as Json value.
        \* ,_pendingJson |->
        \*     LET J == INSTANCE Json
        \*     IN J!ToJson(pending)
        
        \* Lastly, you may build expressions over arbitrary sets of states by
        \* leveraging the _TETrace operator.  For example, this is how to
        \* count the number of times a spec variable changed up to the current
        \* state in the trace.
        \* ,_pendingModCount |->
        \*     LET F[s \in DOMAIN _TETrace] ==
        \*         IF s = 1 THEN 0
        \*         ELSE IF _TETrace[s].pending # _TETrace[s-1].pending
        \*             THEN 1 + F[s-1] ELSE F[s-1]
        \*     IN F[_TEPosition - 1]
    ]

=============================================================================



Parsing and semantic processing can take forever if the trace below is long.
 In this case, it is advised to uncomment the module below to deserialize the
 trace from a generated binary file.

\*
\*---- MODULE ActivationMutant_TETrace ----
\*EXTENDS IOUtils, TLC, ActivationMutant
\*
\*trace == IODeserialize("ActivationMutant_TTrace_1788632505.bin", TRUE)
\*
\*=============================================================================
\*

---- MODULE ActivationMutant_TETrace ----
EXTENDS TLC, ActivationMutant

trace == 
    <<
    ([phase |-> "idle",current |-> NONE,pending |-> NONE,activated |-> {}]),
    ([phase |-> "pending-written",current |-> NONE,pending |-> NONE,activated |-> {}]),
    ([phase |-> "flipped-nopending",current |-> 1,pending |-> NONE,activated |-> {}]),
    ([phase |-> "idle",current |-> 1,pending |-> NONE,activated |-> {}])
    >>
----


=============================================================================

---- CONFIG ActivationMutant_TTrace_1788632505 ----
CONSTANTS
    GENS = { 1 , 2 }
    NONE = NONE
    NONE = NONE

INVARIANT
    _inv

CHECK_DEADLOCK
    \* CHECK_DEADLOCK off because of PROPERTY or INVARIANT above.
    FALSE

INIT
    _init

NEXT
    _next

CONSTANT
    _TETrace <- _trace

ALIAS
    _expression
=============================================================================
\* Generated on Sat Sep 05 18:21:46 UTC 2026