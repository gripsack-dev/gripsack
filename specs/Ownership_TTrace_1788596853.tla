---- MODULE Ownership_TTrace_1788596853 ----
EXTENDS Sequences, Ownership, TLCExt, Toolbox, Naturals, TLC

_expression ==
    LET Ownership_TEExpression == INSTANCE Ownership_TEExpression
    IN Ownership_TEExpression!expression
----

_trace ==
    LET Ownership_TETrace == INSTANCE Ownership_TETrace
    IN Ownership_TETrace!trace
----

_inv ==
    ~(
        TLCGet("level") = Len(_TETrace)
        /\
        declared = (TRUE)
        /\
        desired = ("repoA")
        /\
        prevDeclared = (TRUE)
        /\
        lastAction = ("apply")
        /\
        manifest = ([hash |-> "foreign", preserved |-> TRUE])
        /\
        origin = ("none")
        /\
        prevLive = ("foreign")
        /\
        prevManifest = ("none")
        /\
        live = ("foreign")
        /\
        prevOrigin = ("none")
    )
----

_init ==
    /\ live = _TETrace[1].live
    /\ prevManifest = _TETrace[1].prevManifest
    /\ declared = _TETrace[1].declared
    /\ origin = _TETrace[1].origin
    /\ desired = _TETrace[1].desired
    /\ prevOrigin = _TETrace[1].prevOrigin
    /\ prevLive = _TETrace[1].prevLive
    /\ prevDeclared = _TETrace[1].prevDeclared
    /\ lastAction = _TETrace[1].lastAction
    /\ manifest = _TETrace[1].manifest
----

_next ==
    /\ \E i,j \in DOMAIN _TETrace:
        /\ \/ /\ j = i + 1
              /\ i = TLCGet("level")
        /\ live  = _TETrace[i].live
        /\ live' = _TETrace[j].live
        /\ prevManifest  = _TETrace[i].prevManifest
        /\ prevManifest' = _TETrace[j].prevManifest
        /\ declared  = _TETrace[i].declared
        /\ declared' = _TETrace[j].declared
        /\ origin  = _TETrace[i].origin
        /\ origin' = _TETrace[j].origin
        /\ desired  = _TETrace[i].desired
        /\ desired' = _TETrace[j].desired
        /\ prevOrigin  = _TETrace[i].prevOrigin
        /\ prevOrigin' = _TETrace[j].prevOrigin
        /\ prevLive  = _TETrace[i].prevLive
        /\ prevLive' = _TETrace[j].prevLive
        /\ prevDeclared  = _TETrace[i].prevDeclared
        /\ prevDeclared' = _TETrace[j].prevDeclared
        /\ lastAction  = _TETrace[i].lastAction
        /\ lastAction' = _TETrace[j].lastAction
        /\ manifest  = _TETrace[i].manifest
        /\ manifest' = _TETrace[j].manifest

\* Uncomment the ASSUME below to write the states of the error trace
\* to the given file in Json format. Note that you can pass any tuple
\* to `JsonSerialize`. For example, a sub-sequence of _TETrace.
    \* ASSUME
    \*     LET J == INSTANCE Json
    \*         IN J!JsonSerialize("Ownership_TTrace_1788596853.json", _TETrace)

=============================================================================

 Note that you can extract this module `Ownership_TEExpression`
  to a dedicated file to reuse `expression` (the module in the 
  dedicated `Ownership_TEExpression.tla` file takes precedence 
  over the module `Ownership_TEExpression` below).

---- MODULE Ownership_TEExpression ----
EXTENDS Sequences, Ownership, TLCExt, Toolbox, Naturals, TLC

expression == 
    [
        \* To hide variables of the `Ownership` spec from the error trace,
        \* remove the variables below.  The trace will be written in the order
        \* of the fields of this record.
        live |-> live
        ,prevManifest |-> prevManifest
        ,declared |-> declared
        ,origin |-> origin
        ,desired |-> desired
        ,prevOrigin |-> prevOrigin
        ,prevLive |-> prevLive
        ,prevDeclared |-> prevDeclared
        ,lastAction |-> lastAction
        ,manifest |-> manifest
        
        \* Put additional constant-, state-, and action-level expressions here:
        \* ,_stateNumber |-> _TEPosition
        \* ,_liveUnchanged |-> live = live'
        
        \* Format the `live` variable as Json value.
        \* ,_liveJson |->
        \*     LET J == INSTANCE Json
        \*     IN J!ToJson(live)
        
        \* Lastly, you may build expressions over arbitrary sets of states by
        \* leveraging the _TETrace operator.  For example, this is how to
        \* count the number of times a spec variable changed up to the current
        \* state in the trace.
        \* ,_liveModCount |->
        \*     LET F[s \in DOMAIN _TETrace] ==
        \*         IF s = 1 THEN 0
        \*         ELSE IF _TETrace[s].live # _TETrace[s-1].live
        \*             THEN 1 + F[s-1] ELSE F[s-1]
        \*     IN F[_TEPosition - 1]
    ]

=============================================================================



Parsing and semantic processing can take forever if the trace below is long.
 In this case, it is advised to uncomment the module below to deserialize the
 trace from a generated binary file.

\*
\*---- MODULE Ownership_TETrace ----
\*EXTENDS IOUtils, Ownership, TLC
\*
\*trace == IODeserialize("Ownership_TTrace_1788596853.bin", TRUE)
\*
\*=============================================================================
\*

---- MODULE Ownership_TETrace ----
EXTENDS Ownership, TLC

trace == 
    <<
    ([declared |-> TRUE,desired |-> "repoA",prevDeclared |-> TRUE,lastAction |-> "init",manifest |-> "none",origin |-> "none",prevLive |-> "foreign",prevManifest |-> "none",live |-> "foreign",prevOrigin |-> "none"]),
    ([declared |-> TRUE,desired |-> "repoA",prevDeclared |-> TRUE,lastAction |-> "apply",manifest |-> [hash |-> "foreign", preserved |-> TRUE],origin |-> "none",prevLive |-> "foreign",prevManifest |-> "none",live |-> "foreign",prevOrigin |-> "none"])
    >>
----


=============================================================================

---- CONFIG Ownership_TTrace_1788596853 ----
CONSTANTS
    C0 = "foreign"
    C1 = "repoA"
    C2 = "repoB"
    C3 = "user"

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
\* Generated on Sat Sep 05 08:27:33 UTC 2026