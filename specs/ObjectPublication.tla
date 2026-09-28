---- MODULE ObjectPublication ----
EXTENDS Integers

CONSTANTS ByteValues, ModeValues, OldBytes, NewBytes, OldMode, NewMode, Observe, Mutant
ASSUME Parameters ==
    /\ OldBytes \in ByteValues /\ NewBytes \in ByteValues
    /\ OldMode \in ModeValues /\ NewMode \in ModeValues
    /\ Observe \in BOOLEAN
    /\ Mutant \in {"none", "file_sync", "mode_after_sync", "parent_sync"}

PayloadSpace == [bytes: ByteValues, mode: ModeValues]
OldPayload == [bytes |-> OldBytes, mode |-> OldMode]
NewPayload == [bytes |-> NewBytes, mode |-> NewMode]
Phases == {"write", "mode", "file", "lateMode", "publish", "directory", "return", "done", "stopped"}

VARIABLES cached, durable, cachedName, durableName, phase, published, returned
vars == <<cached, durable, cachedName, durableName, phase, published, returned>>

\* An observed cache hit has admitted bytes but no assumed durability. A new
\* writer may expose its temporary inode only after bytes AND mode are synced.
Init ==
    /\ cached = IF Observe THEN NewPayload ELSE OldPayload
    /\ IF Observe THEN durable \in PayloadSpace ELSE durable = OldPayload
    /\ cachedName = Observe
    /\ IF Observe THEN durableName \in BOOLEAN ELSE durableName = FALSE
    /\ phase = IF Observe THEN "file" ELSE "write"
    /\ published = FALSE /\ returned = FALSE

WriteBytes ==
    /\ phase = "write"
    /\ cached' = [cached EXCEPT !.bytes = NewBytes] /\ phase' = "mode"
    /\ UNCHANGED <<durable, cachedName, durableName, published, returned>>
SetMode ==
    /\ phase = "mode"
    /\ cached' = IF Mutant = "mode_after_sync" THEN cached ELSE [cached EXCEPT !.mode = NewMode]
    /\ phase' = "file"
    /\ UNCHANGED <<durable, cachedName, durableName, published, returned>>
SyncFile ==
    /\ phase = "file"
    /\ durable' = IF Mutant = "file_sync" THEN durable ELSE cached
    /\ phase' = IF Mutant = "mode_after_sync" /\ ~cachedName THEN "lateMode"
                ELSE IF cachedName THEN "directory" ELSE "publish"
    /\ UNCHANGED <<cached, cachedName, durableName, published, returned>>
LateMode ==
    /\ phase = "lateMode"
    /\ cached' = [cached EXCEPT !.mode = NewMode] /\ phase' = "publish"
    /\ UNCHANGED <<durable, cachedName, durableName, published, returned>>
PublishName ==
    /\ phase = "publish"
    /\ cachedName' = TRUE /\ published' = TRUE /\ phase' = "directory"
    /\ UNCHANGED <<cached, durable, durableName, returned>>
SyncParent ==
    /\ phase = "directory"
    /\ durableName' = IF Mutant = "parent_sync" THEN durableName ELSE cachedName
    /\ phase' = "return"
    /\ UNCHANGED <<cached, durable, cachedName, published, returned>>
ReturnAuthority ==
    /\ phase = "return"
    /\ returned' = TRUE /\ phase' = "done"
    /\ UNCHANGED <<cached, durable, cachedName, durableName, published>>

\* Failure/process death does not flush caches or grant completion authority.
Stop ==
    /\ phase' = "stopped"
    /\ UNCHANGED <<cached, durable, cachedName, durableName, published, returned>>
ObserveAgain ==
    /\ phase = "stopped" /\ cachedName /\ cached = NewPayload
    /\ phase' = "file"
    /\ UNCHANGED <<cached, durable, cachedName, durableName, published, returned>>
WritebackData ==
    /\ \E bytes \in {cached.bytes, durable.bytes}, mode \in {cached.mode, durable.mode} :
          durable' = [bytes |-> bytes, mode |-> mode]
    /\ UNCHANGED <<cached, cachedName, durableName, phase, published, returned>>
WritebackName ==
    /\ durableName' \in {cachedName, durableName}
    /\ UNCHANGED <<cached, durable, cachedName, phase, published, returned>>
PowerLoss ==
    /\ \E bytes \in {cached.bytes, durable.bytes}, mode \in {cached.mode, durable.mode} :
          /\ durable' = [bytes |-> bytes, mode |-> mode] /\ cached' = durable'
    /\ durableName' \in {cachedName, durableName} /\ cachedName' = durableName'
    /\ phase' = "stopped"
    /\ UNCHANGED <<published, returned>>

Next == WriteBytes \/ SetMode \/ SyncFile \/ LateMode \/ PublishName \/ SyncParent
        \/ ReturnAuthority \/ Stop \/ ObserveAgain \/ WritebackData \/ WritebackName \/ PowerLoss
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ cached \in PayloadSpace /\ durable \in PayloadSpace
    /\ cachedName \in BOOLEAN /\ durableName \in BOOLEAN
    /\ published \in BOOLEAN /\ returned \in BOOLEAN /\ phase \in Phases
PublishedObjectHasDurablePayload == published => cached = NewPayload /\ durable = NewPayload
AuthorityHasDurableObject == returned =>
    /\ cachedName /\ durableName /\ cached = NewPayload /\ durable = NewPayload
ControlInvariant ==
    /\ phase # "lateMode"
    /\ (phase = "write" =>
          /\ ~cachedName /\ ~durableName /\ ~published /\ ~returned
          /\ cached = OldPayload /\ durable = OldPayload)
    /\ (phase = "mode" => cached.bytes = NewBytes /\ ~cachedName /\ ~published)
    /\ (phase = "file" /\ ~cachedName => ~published)
    /\ (phase \in {"file", "publish", "directory", "return", "done"} => cached = NewPayload)
    /\ (phase \in {"publish", "directory", "return", "done"} => durable = NewPayload)
    /\ (phase = "publish" => ~cachedName /\ ~published)
    /\ (phase \in {"directory", "return", "done"} => cachedName)
    /\ (phase \in {"return", "done"} => durableName)
Invariant == TypeOK /\ ControlInvariant /\ PublishedObjectHasDurablePayload /\ AuthorityHasDurableObject

=============================================================================
