---- MODULE TransactionProofs ----
EXTENDS Transaction, TLAPS

SavedMarker == [prev |-> PREV, target |-> TARGET]
SavedEntry == [prior |-> StartContent, intended |-> Intended]

\* The existing TLC pilot uses two distinct natural generation identities,
\* disjoint option tags, and one post-crash edit outside either owned value.
\* These are admission/model-domain assumptions, not the safety conclusion.
ASSUME Parameters ==
    /\ PREV \in Nat
    /\ TARGET \in Nat
    /\ PREV # TARGET
    /\ NONE \notin Nat
    /\ NONE # SavedMarker
    /\ NONE # SavedEntry
    /\ KIND \in {"deploy", "prune"}
    /\ CEDITED \notin {StartContent, AfterMutate, Intended, ABSENT}

Image(i) ==
    [dest |-> IF i < 3 THEN StartContent ELSE AfterMutate,
     current |-> IF i < 4 THEN PREV ELSE TARGET,
     entry |-> IF i \in 2..4 THEN SavedEntry ELSE NoEntry,
     marker |-> IF i \in 1..5 THEN SavedMarker ELSE NoMarker]

Cuts(i) == {Image(i), Image(i + 1)}
EditedImage(d) == [d EXCEPT !.dest = CEDITED]
RecoveryImages(i, wasEdited) ==
    IF wasEdited THEN {EditedImage(Image(i)), EditedImage(Image(i + 1))}
    ELSE Cuts(i)

THEOREM EntryType ==
    SavedEntry \in [prior: CONTENTS, intended: CONTENTS \union {REMOVED}]
  BY SMT DEF SavedEntry, StartContent, Intended, CONTENTS, ABSENT, REMOVED

THEOREM RecordImage == Effect(1, Image(1)) = Image(2)
  BY SMT DEF Effect, Image, SavedEntry, SavedMarker, StartContent, AfterMutate,
             Intended, NoEntry, NoMarker

\* A completed protocol step grants authority only after its barrier.
THEOREM CompletedStepDurability == DoStep => durable' = volatile'
  BY SMT DEF DoStep


Observed(i, wasEdited) ==
    IF wasEdited THEN EditedImage(Image(i)) ELSE Image(i)
ClassAt(i) ==
    IF i = 0 \/ i = 6 THEN "none"
    ELSE IF i < 4 THEN "uncommitted" ELSE "committed"
ExpectedRecovery(i, wasEdited) ==
    [dest |-> IF wasEdited THEN CEDITED
              ELSE IF i < 4 THEN StartContent ELSE AfterMutate,
     current |-> IF i < 4 THEN PREV ELSE TARGET,
     entry |-> NoEntry, marker |-> NoMarker]

THEOREM RecoveryOfEntry ==
    \A live, prior, intended :
       (/\ NONE # [prior |-> prior, intended |-> intended]
        /\ NONE # SavedMarker
        /\ PREV # TARGET)
       => RecoveredDisk([dest |-> live, current |-> PREV,
                          entry |-> [prior |-> prior, intended |-> intended],
                          marker |-> SavedMarker])
          = [dest |-> IF Decide(live, intended, prior) = "restore" THEN prior ELSE live,
             current |-> PREV, entry |-> NoEntry, marker |-> NoMarker]
  BY IsaT(60) DEF RecoveredDisk, RecoveryClass, Classify, SavedMarker, NoEntry, NoMarker

THEOREM UndoIntended ==
    (IF Decide(AfterMutate, Intended, StartContent) = "restore"
     THEN StartContent ELSE AfterMutate) = StartContent
  BY SMT DEF Decide, AfterMutate, Intended, StartContent, ABSENT, REMOVED

THEOREM RecoveryProjection ==
    \A i \in 0..6, wasEdited \in BOOLEAN :
       /\ RecoveryClass(Observed(i, wasEdited)) = ClassAt(i)
       /\ RecoveredDisk(Observed(i, wasEdited)) = ExpectedRecovery(i, wasEdited)
<1> TAKE i \in 0..6, wasEdited \in BOOLEAN
<1> USE DEF Parameters, RecoveryClass, RecoveredDisk, Classify, Decide,
           Observed, ClassAt, ExpectedRecovery, EditedImage, Image, SavedEntry,
           SavedMarker, StartContent, AfterMutate, Intended, ABSENT, REMOVED,
           NoEntry, NoMarker
<1>1. CASE i = 0
  BY SMT, Parameters, <1>1
<1>2. CASE i = 1
  BY SMT, Parameters, <1>2
<1>3. CASE i = 2
  BY SMT, Parameters, <1>3
<1>4. CASE i = 3
  <2>1. CASE wasEdited
    BY SMT, Parameters, <1>4, <2>1
  <2>2. CASE ~wasEdited
    <3>a. Observed(i, wasEdited) =
            [dest |-> AfterMutate, current |-> PREV,
             entry |-> SavedEntry, marker |-> SavedMarker]
      BY SMT, <1>4, <2>2
    <3>b. RecoveryClass(Observed(i, wasEdited)) = "uncommitted"
      BY SMT, Parameters, <1>4, <2>2
    <3>c. RecoveredDisk(Observed(i, wasEdited)) =
            [dest |-> IF Decide(AfterMutate, Intended, StartContent) = "restore"
                      THEN StartContent ELSE AfterMutate,
             current |-> PREV, entry |-> NoEntry, marker |-> NoMarker]
      <4> HIDE DEF RecoveredDisk, Observed, Decide
      <4> QED
        BY ONLY SMT, Parameters, RecoveryOfEntry, <3>a
    <3> HIDE DEF RecoveredDisk, RecoveryClass, Decide
    <3> QED
      BY SMT, Parameters, <1>4, <2>2, <3>a, <3>b, <3>c, UndoIntended
  <2>3. QED
    BY <2>1, <2>2
<1>5. CASE i = 4
  BY SMT, Parameters, <1>5
<1>6. CASE i = 5
  BY SMT, Parameters, <1>6
<1>7. CASE i = 6
  BY SMT, Parameters, <1>7
<1>8. QED
  BY SMT, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7

Inv ==
    /\ TypeOK
    /\ durable = Image(step)
    /\ volatile \in Cuts(step)
    /\ (phase = "running" =>
          /\ step < 6
          /\ volatile = durable
          /\ visible = Image(0)
          /\ beforeRecover = visible
          /\ edited = FALSE
          /\ klass = "none")
    /\ (phase = "crashed" =>
          /\ step < 6
          /\ visible \in Cuts(step)
          /\ beforeRecover = visible
          /\ edited = FALSE
          /\ klass = "none")
    /\ (phase = "recovering" =>
          /\ step < 6
          /\ visible \in RecoveryImages(step, edited)
          /\ beforeRecover = visible
          /\ klass = "none")
    /\ Oracle
    /\ CleanRunCommits

THEOREM InitImpliesInv == Init => Inv
  BY SMT, Parameters
  DEF Init, Inv, TypeOK, DiskSpace, Image, Cuts, StartContent, AfterMutate,
      Intended, CONTENTS, ABSENT, REMOVED, NoEntry, NoMarker, NoCurrent,
      Oracle, CleanRunCommits, SavedEntry, SavedMarker, RecoveryImages, EditedImage

THEOREM InductiveStep == Inv /\ Next => Inv'
<1> SUFFICES ASSUME Inv, Next PROVE Inv'
  OBVIOUS
<1> USE EntryType, RecordImage
<1> USE DEF Parameters, Inv, TypeOK, DiskSpace, Image, Cuts, RecoveryImages,
           EditedImage, StartContent, AfterMutate, Intended, CONTENTS,
           ABSENT, REMOVED, NoEntry, NoMarker, NoCurrent, Oracle,
           CleanRunCommits, SavedEntry, SavedMarker
<1>1. CASE DoStep
  <2> USE DEF DoStep, Effect
  <2>1. CASE step = 0
    BY SMT, Parameters, <1>1, <2>1
  <2>2. CASE step = 1
    <3>a. volatile = Image(1)
      BY SMT, <1>1, <2>2
    <3>b. /\ volatile' = Image(2)
          /\ durable' = Image(2)
          /\ step' = 2
      <4> HIDE DEF Effect, Image
      <4> QED
        BY ONLY SMT, <1>1, <2>2, <3>a, RecordImage
    <3>1. TypeOK'
      BY SMT, Parameters, <1>1, <2>2, <3>b
    <3>2. durable' = Image(step')
      BY SMT, <3>b
    <3>3. volatile' \in Cuts(step')
      BY SMT, <3>b
    <3>4. /\ phase' = "running"
          /\ step' = 2
          /\ volatile' = durable'
          /\ visible' = Image(0)
          /\ beforeRecover' = visible'
          /\ edited' = FALSE
          /\ klass' = "none"
      BY SMT, Parameters, <1>1, <2>2
    <3>5. Oracle' /\ CleanRunCommits'
      BY SMT, <3>4
    <3>6. QED
      BY <3>1, <3>2, <3>3, <3>4, <3>5
  <2>3. CASE step = 2
    BY SMT, Parameters, <1>1, <2>3
  <2>4. CASE step = 3
    BY SMT, Parameters, <1>1, <2>4
  <2>5. CASE step = 4
    BY SMT, Parameters, <1>1, <2>5
  <2>6. CASE step = 5
    BY SMT, Parameters, <1>1, <2>6
  <2>7. QED
    BY SMT, <1>1, <2>1, <2>2, <2>3, <2>4, <2>5, <2>6
<1>2. CASE CrashKill
  BY SMT, Parameters, <1>2 DEF CrashKill
<1>3. CASE CrashMidStepKill
  <2> USE DEF CrashMidStepKill, Effect
  <2>1. CASE step = 0
    BY SMT, Parameters, <1>3, <2>1
  <2>2. CASE step = 1
    <3>a. volatile = Image(1)
      BY SMT, <1>3, <2>2
    <3>b. Effect(step, volatile) = Image(2)
      <4> HIDE DEF Effect, Image
      <4> QED
        BY ONLY SMT, <2>2, <3>a, RecordImage
    <3> HIDE DEF Effect
    <3> QED
      BY SMT, Parameters, <1>3, <2>2, <3>b
  <2>3. CASE step = 2
    BY SMT, Parameters, <1>3, <2>3
  <2>4. CASE step = 3
    BY SMT, Parameters, <1>3, <2>4
  <2>5. CASE step = 4
    BY SMT, Parameters, <1>3, <2>5
  <2>6. CASE step = 5
    BY SMT, Parameters, <1>3, <2>6
  <2>7. QED
    BY SMT, <1>3, <2>1, <2>2, <2>3, <2>4, <2>5, <2>6
<1>4. CASE CrashPower
  BY SMT, Parameters, <1>4 DEF CrashPower
<1>5. CASE CrashMidStepPower
  <2> USE DEF CrashMidStepPower, Effect
  <2>1. CASE step = 0
    BY SMT, Parameters, <1>5, <2>1
  <2>2. CASE step = 1
    <3>a. volatile = Image(1)
      BY SMT, <1>5, <2>2
    <3>b. Effect(step, volatile) = Image(2)
      <4> HIDE DEF Effect, Image
      <4> QED
        BY ONLY SMT, <2>2, <3>a, RecordImage
    <3> HIDE DEF Effect
    <3> QED
      BY SMT, Parameters, <1>5, <2>2, <3>b
  <2>3. CASE step = 2
    BY SMT, Parameters, <1>5, <2>3
  <2>4. CASE step = 3
    BY SMT, Parameters, <1>5, <2>4
  <2>5. CASE step = 4
    BY SMT, Parameters, <1>5, <2>5
  <2>6. CASE step = 5
    BY SMT, Parameters, <1>5, <2>6
  <2>7. QED
    BY SMT, <1>5, <2>1, <2>2, <2>3, <2>4, <2>5, <2>6
<1>6. CASE MaybeUserEdit
  BY SMT, Parameters, <1>6 DEF MaybeUserEdit
<1>7. CASE Recover
  <2>1. PICK i \in 0..6 : visible = Observed(i, edited)
    BY SMT, <1>7 DEF Recover, Observed
  <2>2. edited \in BOOLEAN
    BY DEF Inv, TypeOK
  <2>3. /\ klass' = ClassAt(i)
        /\ visible' = ExpectedRecovery(i, edited)
    <3> HIDE DEF Observed, Image, ClassAt, ExpectedRecovery,
                 RecoveryClass, RecoveredDisk
    <3> QED
      BY ONLY SMT, <1>7, <2>1, <2>2, RecoveryProjection DEF Recover
  <2>4. QED
    BY SMT, Parameters, <1>7, <2>1, <2>2, <2>3
    DEF Recover, Observed, ClassAt, ExpectedRecovery
<1>8. CASE phase = "done" /\ UNCHANGED vars
  BY <1>8 DEF vars
<1>9. QED
  BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8 DEF Next

THEOREM InvImpliesSafety == Inv => Oracle /\ CleanRunCommits
  BY DEF Inv

THEOREM TransactionSafety == Spec => [](Oracle /\ CleanRunCommits)
<1>1. Inv /\ UNCHANGED vars => Inv'
  BY DEF Inv, TypeOK, Oracle, CleanRunCommits, vars
<1>2. Spec => []Inv
  BY PTL, InitImpliesInv, InductiveStep, <1>1 DEF Spec
<1>3. QED
  BY PTL, <1>2, InvImpliesSafety
=============================================================================
