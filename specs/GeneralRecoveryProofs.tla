---- MODULE GeneralRecoveryProofs ----
EXTENDS LifecycleRetentionProofs

GeneralRecoveryContract ==
    /\ RecoveryEvidencePreserved /\ MutationHasDurableMarker /\ ExactCommitIdentity
    /\ CurrentNamesDurableGeneration /\ AllocationHistoryCovered /\ NoProtectedRootCollection
    /\ EveryPlanHasDurablePreparation
    /\ Hooks!OutcomeAfterReturn /\ Hooks!TerminalNoReplay /\ Hooks!EffectsBindFullSelection
    /\ Hooks!ArchiveBeforeClear /\ Hooks!NoSilentSkip

THEOREM CommitClassificationUsesExactIdentity == ExactCommitIdentity
  BY SMT, CorrectJournal DEF ExactCommitIdentity, Classification, Selection!Classify

THEOREM JoinedInvariantImpliesRecoveryContract == RetentionLifecycleInvariant => GeneralRecoveryContract
  BY SMT, JournalInvariantPreservesRecoveryEvidence, CommitClassificationUsesExactIdentity,
      PrunedGenerationIdsRemainCovered
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant,
      LifecycleInvariant, GeneralRecoveryContract, Hooks!ActivationInductiveInvariant,
      Hooks!HomeInvariant, Hooks!HistoryInvariant, Hooks!ArchiveInvariant, Hooks!ArchiveBeforeClear

THEOREM ArbitraryFiniteRepeatedRecoverySafety == RetentionLifecycleSpec => []GeneralRecoveryContract
  BY PTL, GeneralLifecycleRetentionSafety, JoinedInvariantImpliesRecoveryContract

THEOREM JoinedEntryCleanupRequiresStableRestoration ==
  ASSUME NEW destination \in Destinations, RetentionLifecycleInvariant, ~Committed, RemoveEntry(destination)
  PROVE ~UnsafeOwned(cells[destination], cells[destination].durable.live)
  BY SMT, EntryRemovalRequiresDurableRestoration
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant, ActivationLifecycleInvariant, LifecycleInvariant

THEOREM JoinedHookInvocationBindsDurableSelection ==
  ASSUME RetentionLifecycleInvariant, Hooks!Invoke
  PROVE currentVisible = <<active, plans[active]>> /\ currentStable = currentVisible
  BY SMT, InvocationUsesDurableFullSelection
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant

THEOREM JoinedPendingResurrectionIsSettled ==
  ASSUME RetentionLifecycleInvariant, pendingC = ActivationNone, pendingD # ActivationNone
  PROVE pendingD \in archiveD /\ Hooks!AllSettled(pendingD, stateD)
  BY SMT, ClearedPendingResurrectionHasNoUnsettledIntents
  DEF RetentionLifecycleInvariant, PreparedLifecycleInvariant

THEOREM RejectedAdmissionCannotRemoveVisiblePayloads ==
  ASSUME RetentionTypes, RetentionLifecycleNext(FALSE)
  PROVE payloadsC \subseteq payloadsC'
  BY SMT DEF RetentionTypes, RetentionLifecycleNext, AcquireLifecycle, LifecycleOperation, MetadataEnvironment,
      PayloadWriteback, ProcessInterruption, StorageInterruption, payloadVars

=============================================================================
