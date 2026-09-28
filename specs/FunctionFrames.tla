---- MODULE FunctionFrames ----
EXTENDS Integers, FiniteSets, TLAPS

ReplaceCell(state, destination, cell) == [state EXCEPT ![destination] = cell]

THEOREM ReplacementDomain ==
  ASSUME NEW Destinations, NEW Values,
         NEW state \in [Destinations -> Values],
         NEW destination \in Destinations, NEW cell \in Values
  PROVE ReplaceCell(state, destination, cell) \in [Destinations -> Values]
  BY SMT DEF ReplaceCell

THEOREM ReplacementFrame ==
  ASSUME NEW Destinations, NEW Values,
         NEW state \in [Destinations -> Values],
         NEW destination \in Destinations, NEW cell \in Values
  PROVE /\ ReplaceCell(state, destination, cell)[destination] = cell
        /\ \A other \in Destinations \ {destination} :
             ReplaceCell(state, destination, cell)[other] = state[other]
  BY SMT DEF ReplaceCell

THEOREM IndependentReplacementsCommute ==
  ASSUME NEW Destinations, NEW Values,
         NEW state \in [Destinations -> Values],
         NEW first \in Destinations, NEW second \in Destinations,
         first # second, NEW firstValue \in Values, NEW secondValue \in Values
  PROVE ReplaceCell(ReplaceCell(state, first, firstValue), second, secondValue)
      = ReplaceCell(ReplaceCell(state, second, secondValue), first, firstValue)
  BY SMT DEF ReplaceCell

THEOREM PointwiseInvariantFrame ==
  ASSUME NEW Destinations, NEW Values,
         NEW state \in [Destinations -> Values],
         NEW destination \in Destinations, NEW cell \in Values,
         NEW Inv(_,_),
         \A item \in Destinations : Inv(item, state[item]),
         Inv(destination, cell)
  PROVE \A item \in Destinations : Inv(item, ReplaceCell(state, destination, cell)[item])
  BY SMT DEF ReplaceCell

=============================================================================
