---- MODULE PreparationPublication ----
EXTENDS Integers, FiniteSets

CONSTANTS Documents, Depth, Mutant, NamespaceMutant
ASSUME Parameters ==
    /\ IsFiniteSet(Documents) /\ Depth \in Nat
    /\ Mutant \in {"none", "file_sync", "mode_after_sync", "parent_sync", "incomplete_set"}

VARIABLES nodes, names, cachedReachable, namespaceCursor, namespacePhase, namespaceReturned
Namespace == INSTANCE NamespaceSealing
    WITH cursor <- namespaceCursor, phase <- namespacePhase, returned <- namespaceReturned,
         Mutant <- NamespaceMutant
ASSUME NamespaceDomain == Namespace!Parameters
namespaceVars == <<nodes, names, cachedReachable, namespaceCursor, namespacePhase, namespaceReturned>>

DocumentPhases == {"write", "mode", "file", "publish", "directory", "done", "stopped"}
VARIABLES bytesC, bytesD, modesC, modesD, namesC, namesD, documentPhase, stopped, returned, requested
publicationVars == <<bytesC, bytesD, modesC, modesD, namesC, namesD, documentPhase, stopped, returned>>
vars == <<namespaceVars, publicationVars, requested>>

\* A preparation freezes its complete admitted plan/outcome set. The universe
\* also permits successive transactions with different intent counts; changing
\* the requested set requires a new initialization, never an in-flight shortcut.
InitFor(required) ==
    /\ required \subseteq Documents /\ requested = required /\ Namespace!Init
    /\ bytesC = {} /\ bytesD = {} /\ modesC = {} /\ modesD = {}
    /\ namesC = {} /\ namesD = {}
    /\ documentPhase = [document \in Documents |-> "write"]
    /\ stopped = FALSE /\ returned = FALSE
Init == InitFor(Documents)

ResetFor(required) ==
    /\ required \subseteq Documents /\ requested' = required /\ Namespace!Reset
    /\ bytesC' = {} /\ bytesD' = {} /\ modesC' = {} /\ modesD' = {}
    /\ namesC' = {} /\ namesD' = {}
    /\ documentPhase' = [document \in Documents |-> "write"]
    /\ stopped' = FALSE /\ returned' = FALSE

WriteBytes(document) ==
    /\ ~stopped /\ cachedReachable /\ documentPhase[document] = "write"
    /\ bytesC' = bytesC \union {document}
    /\ documentPhase' = [documentPhase EXCEPT ![document] = "mode"]
    /\ UNCHANGED <<bytesD, modesC, modesD, namesC, namesD, stopped, returned, namespaceVars>>
SetPrivateMode(document) ==
    /\ ~stopped /\ cachedReachable /\ documentPhase[document] = "mode"
    /\ modesC' = IF Mutant = "mode_after_sync" THEN modesC ELSE modesC \union {document}
    /\ documentPhase' = [documentPhase EXCEPT ![document] = "file"]
    /\ UNCHANGED <<bytesC, bytesD, modesD, namesC, namesD, stopped, returned, namespaceVars>>
SyncFile(document) ==
    /\ ~stopped /\ cachedReachable /\ documentPhase[document] = "file"
    /\ bytesD' = IF Mutant = "file_sync" THEN bytesD ELSE bytesD \union ({document} \intersect bytesC)
    /\ modesD' = IF Mutant = "file_sync" THEN modesD ELSE modesD \union ({document} \intersect modesC)
    /\ documentPhase' = [documentPhase EXCEPT ![document] = "publish"]
    /\ UNCHANGED <<bytesC, modesC, namesC, namesD, stopped, returned, namespaceVars>>
PublishName(document) ==
    /\ ~stopped /\ cachedReachable /\ documentPhase[document] = "publish"
    /\ namesC' = namesC \union {document}
    /\ modesC' = IF Mutant = "mode_after_sync" THEN modesC \union {document} ELSE modesC
    /\ documentPhase' = [documentPhase EXCEPT ![document] = "directory"]
    /\ UNCHANGED <<bytesC, bytesD, modesD, namesD, stopped, returned, namespaceVars>>
SyncDocumentParent(document) ==
    /\ ~stopped /\ cachedReachable /\ documentPhase[document] = "directory"
    /\ namesD' = IF Mutant = "parent_sync" THEN namesD ELSE namesD \union {document}
    /\ documentPhase' = [documentPhase EXCEPT ![document] = "done"]
    /\ UNCHANGED <<bytesC, bytesD, modesC, modesD, namesC, stopped, returned, namespaceVars>>

SealNamespace ==
    /\ ~stopped /\ (Namespace!SyncDirectory \/ Namespace!ReturnAuthority)
    /\ UNCHANGED publicationVars
ReturnPreparation ==
    /\ ~stopped /\ namespaceReturned
    /\ IF Mutant = "incomplete_set"
       THEN \E document \in requested : documentPhase[document] = "done"
       ELSE \A document \in requested : documentPhase[document] = "done"
    /\ returned' = TRUE
    /\ UNCHANGED <<bytesC, bytesD, modesC, modesD, namesC, namesD, documentPhase, stopped, namespaceVars>>

WritebackDocuments ==
    /\ bytesD' \in {next \in SUBSET bytesC : bytesD \subseteq next}
    /\ modesD' \in {next \in SUBSET modesC : modesD \subseteq next}
    /\ namesD' \in {next \in SUBSET namesC : namesD \subseteq next}
    /\ UNCHANGED <<bytesC, modesC, namesC, documentPhase, stopped, returned, namespaceVars>>
WritebackNamespace == Namespace!Writeback /\ UNCHANGED publicationVars
ProcessDeath ==
    /\ Namespace!ProcessDeath /\ stopped' = TRUE
    /\ documentPhase' = [document \in Documents |-> "stopped"]
    /\ UNCHANGED <<bytesC, bytesD, modesC, modesD, namesC, namesD, returned>>
PowerLoss ==
    /\ Namespace!PowerLoss
    /\ bytesD' \in {next \in SUBSET bytesC : bytesD \subseteq next} /\ bytesC' = bytesD'
    /\ modesD' \in {next \in SUBSET modesC : modesD \subseteq next} /\ modesC' = modesD'
    /\ namesD' \in {next \in SUBSET namesC : namesD \subseteq next} /\ namesC' = namesD'
    /\ stopped' = TRUE /\ documentPhase' = [document \in Documents |-> "stopped"]
    /\ UNCHANGED returned

PublicationStep == (\E document \in requested : WriteBytes(document) \/ SetPrivateMode(document)
          \/ SyncFile(document) \/ PublishName(document) \/ SyncDocumentParent(document))
    \/ SealNamespace \/ ReturnPreparation \/ WritebackDocuments \/ WritebackNamespace \/ ProcessDeath \/ PowerLoss
Next == PublicationStep /\ UNCHANGED requested
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ requested \subseteq Documents
    /\ bytesC \subseteq Documents /\ bytesD \subseteq Documents
    /\ modesC \subseteq Documents /\ modesD \subseteq Documents
    /\ namesC \subseteq Documents /\ namesD \subseteq Documents
    /\ documentPhase \in [Documents -> DocumentPhases] /\ stopped \in BOOLEAN /\ returned \in BOOLEAN
StorageInvariant ==
    /\ bytesD \subseteq bytesC /\ modesD \subseteq modesC /\ namesD \subseteq namesC
    /\ namesC \subseteq bytesD \intersect modesD
ControlInvariant ==
    /\ \A document \in Documents :
          /\ (documentPhase[document] \in {"mode", "file", "publish", "directory", "done"} => document \in bytesC)
          /\ (documentPhase[document] \in {"file", "publish", "directory", "done"} => document \in modesC)
          /\ (documentPhase[document] \in {"publish", "directory", "done"} => document \in bytesD \intersect modesD)
          /\ (documentPhase[document] \in {"directory", "done"} => document \in namesC)
          /\ (documentPhase[document] = "done" => document \in namesD)
    /\ (stopped => \A document \in Documents : documentPhase[document] = "stopped")
PreparedDocumentsAreDurable == returned =>
    /\ requested \subseteq bytesD \intersect modesD \intersect namesD
    /\ Namespace!AllSealed /\ cachedReachable
Invariant == TypeOK /\ StorageInvariant /\ ControlInvariant /\ Namespace!Invariant /\ PreparedDocumentsAreDurable

=============================================================================
