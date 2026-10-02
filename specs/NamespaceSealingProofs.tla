---- MODULE NamespaceSealingProofs ----
EXTENDS NamespaceSealing, TLAPS

ASSUME CorrectProtocol == Mutant = "none"

THEOREM ExtendSealedSuffix ==
  ASSUME NEW upper \in Nat, NEW lower \in Int, NEW known,
         (lower + 1)..upper \subseteq known
  PROVE lower..upper \subseteq known \union {lower}
<1>1. \A value \in lower..upper : value = lower \/ value \in (lower + 1)..upper
  BY SMT
<1>2. QED
  BY SMT, <1>1

THEOREM NamespaceInitialSafety == Init => Invariant
  BY SMT, Parameters, CorrectProtocol
  DEF Init, Invariant, TypeOK, ControlInvariant, AuthorityHasDurableNamespace,
      AllSealed, Names, Phases

THEOREM ResetMatchesInit == Reset = Init'
  BY DEF Reset, Init

THEOREM NamespaceResetSafety == Reset => Invariant'
  BY SMT, Parameters, CorrectProtocol
  DEF Reset, Invariant, TypeOK, ControlInvariant, AuthorityHasDurableNamespace,
      AllSealed, Names, Phases

THEOREM NamespaceInduction == Invariant /\ Next => Invariant'
<1> SUFFICES ASSUME Invariant, Next PROVE Invariant'
  OBVIOUS
<1> USE DEF Invariant, TypeOK, ControlInvariant, AuthorityHasDurableNamespace,
           AllSealed, Names, Phases
<1>1. CASE SyncDirectory
  <2> USE DEF SyncDirectory
  <2>1. CASE cursor = 0 /\ Depth = 0
    BY SMT, Parameters, CorrectProtocol, <1>1, <2>1 DEF SyncDirectory
  <2>2. CASE cursor = 0 /\ Depth > 0
    <3>1. 1..Depth \subseteq names \union {1}
      BY SMT, ExtendSealedSuffix, Parameters, <1>1, <2>2
    <3>2. QED
      BY SMT, Parameters, CorrectProtocol, <1>1, <2>2, <3>1 DEF SyncDirectory
  <2>3. CASE cursor > 0 /\ cursor = Depth
    BY SMT, Parameters, CorrectProtocol, <1>1, <2>3 DEF SyncDirectory
  <2>4. CASE cursor > 0 /\ cursor < Depth
    <3>1. cursor..Depth \subseteq nodes \union {cursor}
      BY SMT, ExtendSealedSuffix, Parameters, <1>1, <2>4
    <3>2. (cursor + 1)..Depth \subseteq names \union {cursor + 1}
      BY SMT, ExtendSealedSuffix, Parameters, <1>1, <2>4
    <3>3. QED
      BY SMT, Parameters, CorrectProtocol, <1>1, <2>4, <3>1, <3>2 DEF SyncDirectory
  <2>5. QED
    BY SMT, Parameters, <2>1, <2>2, <2>3, <2>4
<1>2. CASE ReturnAuthority
  BY SMT, Parameters, CorrectProtocol, <1>2 DEF ReturnAuthority
<1>3. CASE ProcessDeath
  BY SMT, Parameters, CorrectProtocol, <1>3 DEF ProcessDeath
<1>4. CASE ResumeObserved
  BY SMT, Parameters, CorrectProtocol, <1>4 DEF ResumeObserved
<1>5. CASE Writeback
  BY SMT, Parameters, CorrectProtocol, <1>5 DEF Writeback
<1>6. CASE PowerLoss
  BY SMT, Parameters, CorrectProtocol, <1>6 DEF PowerLoss
<1>7. QED
  BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6 DEF Next

THEOREM ArbitraryDepthNamespaceSafety == Spec => []Invariant
<1>1. Init => Invariant
  BY NamespaceInitialSafety
<1>2. Invariant /\ [Next]_vars => Invariant'
  BY NamespaceInduction DEF Invariant, TypeOK, ControlInvariant,
      AuthorityHasDurableNamespace, AllSealed, vars
<1>3. QED
  BY PTL, <1>1, <1>2 DEF Spec

THEOREM DurableNamespaceAuthority == Spec => []AuthorityHasDurableNamespace
  BY PTL, ArbitraryDepthNamespaceSafety DEF Invariant

=============================================================================
