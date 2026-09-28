---- MODULE ObjectPublicationProofs ----
EXTENDS ObjectPublication, TLAPS

ASSUME CorrectProtocol == Mutant = "none"

THEOREM PublicationInitialSafety == Init => Invariant
  BY SMT, Parameters, CorrectProtocol
  DEF Init, Invariant, TypeOK, ControlInvariant, PublishedObjectHasDurablePayload,
      AuthorityHasDurableObject, PayloadSpace, OldPayload, NewPayload, Phases

THEOREM PublicationInduction == Invariant /\ Next => Invariant'
<1> SUFFICES ASSUME Invariant, Next PROVE Invariant'
  OBVIOUS
<1> USE DEF Invariant, TypeOK, ControlInvariant, PublishedObjectHasDurablePayload,
           AuthorityHasDurableObject, PayloadSpace, OldPayload, NewPayload, Phases
<1>1. CASE WriteBytes
  BY SMT, Parameters, CorrectProtocol, <1>1 DEF WriteBytes
<1>2. CASE SetMode
  BY SMT, Parameters, CorrectProtocol, <1>2 DEF SetMode
<1>3. CASE SyncFile
  BY SMT, Parameters, CorrectProtocol, <1>3 DEF SyncFile
<1>4. CASE LateMode
  BY SMT, Parameters, CorrectProtocol, <1>4 DEF LateMode
<1>5. CASE PublishName
  BY SMT, Parameters, CorrectProtocol, <1>5 DEF PublishName
<1>6. CASE SyncParent
  BY SMT, Parameters, CorrectProtocol, <1>6 DEF SyncParent
<1>7. CASE ReturnAuthority
  BY SMT, Parameters, CorrectProtocol, <1>7 DEF ReturnAuthority
<1>8. CASE Stop
  BY SMT, Parameters, CorrectProtocol, <1>8 DEF Stop
<1>9. CASE ObserveAgain
  BY SMT, Parameters, CorrectProtocol, <1>9 DEF ObserveAgain
<1>10. CASE WritebackData
  BY SMT, Parameters, CorrectProtocol, <1>10 DEF WritebackData
<1>11. CASE WritebackName
  BY SMT, Parameters, CorrectProtocol, <1>11 DEF WritebackName
<1>12. CASE PowerLoss
  BY SMT, Parameters, CorrectProtocol, <1>12 DEF PowerLoss
<1>13. QED
  BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12
  DEF Next

THEOREM AuthorityRequiresBothBarriers == Invariant /\ ReturnAuthority =>
    /\ cachedName /\ durableName /\ cached = NewPayload /\ durable = NewPayload
  BY SMT DEF Invariant, ControlInvariant, ReturnAuthority

THEOREM ObjectPublicationSafety == Spec => []Invariant
<1>1. Init => Invariant
  BY PublicationInitialSafety
<1>2. Invariant /\ [Next]_vars => Invariant'
  BY PublicationInduction DEF Invariant, TypeOK, ControlInvariant,
      PublishedObjectHasDurablePayload, AuthorityHasDurableObject, vars
<1>3. QED
  BY PTL, <1>1, <1>2 DEF Spec

THEOREM DurablePublishedAuthority == Spec => []AuthorityHasDurableObject
  BY PTL, ObjectPublicationSafety DEF Invariant

=============================================================================
