#!/bin/sh
# Positive protocols and calibrated counterexamples; parser failures never count.
set -eu
jar=${1:?usage: check_models.sh /path/to/tla2tools.jar}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM

# Models run from a private copy: counterexample exports (TTrace), state
# directories and crash artifacts must never land beside the repo sources.
spec="$work/specs"
mkdir -p "$spec"
cp -R specs/. "$spec/"

check() {
    module=$1
    config=$2
    expected=${3:-clean}
    cfg="$spec/$config"
    heap=${4:-1g}
    workers=${5:-1}
    log="$work/$(basename "$config").log"
    status=0
    (cd "$spec" && java "-Xmx$heap" -XX:+UseParallelGC -cp "$jar" tlc2.TLC -cleanup -workers "$workers" \
        -metadir "$work/states" -config "$config" "$module") >"$log" 2>&1 || status=$?
    case "$expected" in
        clean)
            if [ "$status" -ne 0 ] || ! grep -q 'Model checking completed. No error' "$log" \
                || ! grep -Eq 'Finished computing initial states: [1-9][0-9,]* distinct states? generated' "$log"; then
                cat "$log"; echo "FAIL: $config" >&2; exit 1
            fi
            ;;
        temporal:*)
            property=${expected#temporal:}
            # The negative must name exactly one temporal property, and it
            # must be the intended one: a temporal failure can then never
            # calibrate a different liveness claim by accident.
            # TLC's VIOLATION_LIVENESS exit is 13. The message may omit the
            # property name; the unique declaration above binds that evidence.
            props=$(grep -c '^[[:space:]]*PROPERT' "$cfg")
            named=$(grep '^[[:space:]]*PROPERT' "$cfg" | tr -s ' \t' ' ' | tr -d '\r' \
                | sed 's/^ *//')
            if [ "$props" -ne 1 ] || [ "$named" != "PROPERTY $property" ] \
                || [ "$status" -ne 13 ] \
                || ! grep -Eq 'Temporal propert(y|ies)( .*)? (was|were) violated' "$log"; then
                cat "$log"; echo "FAIL: $config did not solely violate $property" >&2; exit 1
            fi
            ;;
        *)
            # The negative must actually check the named invariant, so the
            # reported violation is the intended calibration.
            if [ "$status" -ne 12 ] || ! tr -s ' \t' '\n' <"$cfg" | grep -qx "$expected" \
                || ! grep -q "Invariant $expected is violated" "$log"; then
                cat "$log"; echo "FAIL: $config did not violate $expected" >&2; exit 1
            fi
            ;;
    esac
    echo "$config: $expected"
}

check Ownership.tla cfg/ownership.cfg
for mode in apply-deploy apply-prune rollback-deploy rollback-prune; do
    check Transaction.tla "cfg/$mode.cfg"
    check RepeatedRecovery.tla "cfg/repeated-$mode.cfg"
done
check RepeatedRecovery.tla cfg/repeated-same-generation.cfg
check RepeatedRecovery.tla cfg/repeated-empty-destinations.cfg
check RepeatedRecovery.tla cfg/repeated-fresh-selection.cfg
check Activation.tla cfg/activation.cfg
check Activation.tla cfg/activation-unsealed-start.cfg PermitAfterDurableStart
check Activation.tla cfg/activation-early-success.cfg OutcomeAfterReturn
check Activation.tla cfg/activation-retry-failure.cfg TerminalNoReplay
check Activation.tla cfg/activation-generation-commit.cfg EffectsBindFullSelection
check Activation.tla cfg/activation-early-clear.cfg ArchiveBeforeClear
check Activation.tla cfg/activation-generation-token.cfg DistinctIntentIdentity
check RepeatedActivation.tla cfg/repeated-activation.cfg
check RepeatedActivation.tla cfg/repeated-activation-repeated-generation.cfg
check RepeatedActivation.tla cfg/repeated-activation-lost-pending.cfg NoSilentSkip
# Journal v2: repeated writes to one destination retain the original prior and
# recognize a durable intermediate state before the next write lands.
check DestinationWrites.tla cfg/destination-writes.cfg
check DestinationWrites.tla cfg/destination-writes-lost-prior.cfg OriginalPriorPreserved
check DestinationWrites.tla cfg/destination-writes-lost-before.cfg NoPartialRecovery
check DestinationWrites.tla cfg/destination-writes-witness.cfg NeverTwoWrites
# Journal v3: actual inode-bound parent/child topology, independent durable
# fields, child-first recovery, fail-closed foreign occupants and old formats.
for case in hierarchy empty filesystem-identity birth-identity filesystem-kind missing-fsid missing-birth v1 v2 completion empty-completion; do
    check DirectoryHierarchy.tla "cfg/directory-$case.cfg"
done
check DirectoryHierarchy.tla cfg/directory-missing-intent.cfg MutationHasDurableIntent
check DirectoryHierarchy.tla cfg/directory-missing-preparation.cfg PreparedIdentityBeforeMutation
check DirectoryHierarchy.tla cfg/directory-ancestor-substitution.cfg ForeignLeavesPreserved
check DirectoryHierarchy.tla cfg/directory-inode-only.cfg AncestorIdentityBound
check DirectoryHierarchy.tla cfg/directory-birth-ignored.cfg AncestorIdentityBound
check DirectoryHierarchy.tla cfg/directory-filesystem-kind-ignored.cfg AncestorIdentityBound
check DirectoryHierarchy.tla cfg/directory-parent-first.cfg ChildBeforeParent
check DirectoryHierarchy.tla cfg/directory-foreign-child.cfg ForeignChildrenPreserved
check DirectoryHierarchy.tla cfg/directory-early-cleanup.cfg RestorationBeforeCleanup
check DirectoryHierarchy.tla cfg/directory-legacy-authority.cfg V3AuthorityOnly
check DirectoryHierarchy.tla cfg/directory-wrong-target.cfg ForeignParentPreserved
check DirectoryHierarchy.tla cfg/directory-install-witness.cfg NeverInstalled
check DirectoryHierarchy.tla cfg/directory-repeat-write-witness.cfg NeverRepeatedWrite
check DirectoryHierarchy.tla cfg/directory-repeat-recovery-witness.cfg NeverInterruptedRecovery
# Reverse cutover retains the empty exact inode, and reverses recovery order:
# reinstall its ancestor before restoring bound leaf priors.
check ReverseDirectoryHierarchy.tla cfg/directory-reverse.cfg
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-empty.cfg
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-child-first.cfg ParentBeforeChildRestore
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-nonempty.cfg EmptyBeforeStaging
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-drop-stage.cfg CommittedStageRetained
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-missing-intent.cfg ReverseMutationHasIntent
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-foreign-stage.cfg ForeignPreserved
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-stage-witness.cfg NeverStaged
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-reactivation-witness.cfg NeverReactivated
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-root-replay.cfg RootUndoNeverReplacesRestoredDirectory
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-parent-before-root.cfg RootNormalizedBeforeParentRestore
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-older-file-witness.cfg NeverOlderFileTarget
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-older-absent-witness.cfg NeverOlderAbsentTarget
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-original-contents-witness.cfg NeverOriginalStageChildRestored
check ReverseDirectoryHierarchy.tla cfg/directory-reverse-target-nonempty.cfg TargetReactivationEmpty
# Retained reactivation preserves the target inode even on rollback. Exchange
# between distinct generation incarnations is not the single-inode model.
check IncarnationExchange.tla cfg/directory-exchange.cfg
check IncarnationExchange.tla cfg/directory-exchange-empty.cfg
check IncarnationExchange.tla cfg/directory-reactivate-absent.cfg
check IncarnationExchange.tla cfg/directory-reactivate-file.cfg
check IncarnationExchange.tla cfg/directory-reactivate-link.cfg
check IncarnationExchange.tla cfg/directory-exchange-destroy-target.cfg RetainedTargetNeverDestroyed
check IncarnationExchange.tla cfg/directory-exchange-nonempty-target.cfg OnlyEmptyTargetStaged
check IncarnationExchange.tla cfg/directory-exchange-wrong-prior.cfg ExactCapturedPrior
check IncarnationExchange.tla cfg/directory-exchange-wrong-incarnation.cfg LeafBindsIncarnation
check IncarnationExchange.tla cfg/directory-exchange-missing-intent.cfg ExchangeIntentBeforeInstall
check IncarnationExchange.tla cfg/directory-exchange-install-witness.cfg NeverTargetInstalled
check IncarnationExchange.tla cfg/directory-exchange-commit-witness.cfg NeverExchangeCommitted
check IncarnationExchange.tla cfg/directory-exchange-incarnations-witness.cfg NeverBothIncarnations
check IncarnationExchange.tla cfg/directory-exchange-restage-witness.cfg NeverTargetRestaged
check IncarnationExchange.tla cfg/directory-exchange-original-contents-witness.cfg NeverOriginalStageChildRestored
check IncarnationExchange.tla cfg/directory-exchange-target-nonempty.cfg OnlyEmptyTargetInstalled
# Ordinary tree growth uses node-level evolution, not a frozen leaf list or
# full-root exchange. Include borrowed ancestry, foreign siblings and history.
for case in grow foreign-sibling remove retarget reverse reverse-borrowed; do
    check DirectoryEvolution.tla "cfg/directory-evolve-$case.cfg"
done
check DirectoryEvolution.tla cfg/directory-evolve-missing-barrier.cfg MutationHasDurableEvolve
check DirectoryEvolution.tla cfg/directory-evolve-ancestor.cfg HierarchyBound
check DirectoryEvolution.tla cfg/directory-evolve-parent-first.cfg ChildBeforeParent
check DirectoryEvolution.tla cfg/directory-evolve-borrowed-delete.cfg BorrowedNeverRemoved
check DirectoryEvolution.tla cfg/directory-evolve-foreign-child.cfg ForeignChildrenPreserved
check DirectoryEvolution.tla cfg/directory-evolve-leaf-identity.cfg ForeignLeavesPreserved
check DirectoryEvolution.tla cfg/directory-evolve-leaf-ancestor.cfg OldLeavesAfterShapeRestore
check DirectoryEvolution.tla cfg/directory-evolve-discard-published.cfg PublishedIdentitiesRetained
check DirectoryEvolution.tla cfg/directory-evolve-grow-witness.cfg NeverGrewNested
check DirectoryEvolution.tla cfg/directory-evolve-remove-witness.cfg NeverRemovedOld
check DirectoryEvolution.tla cfg/directory-evolve-retarget-witness.cfg NeverRetargeted
check DirectoryEvolution.tla cfg/directory-evolve-foreign-commit-witness.cfg NeverForeignSiblingCommit
check DirectoryEvolution.tla cfg/directory-evolve-history-witness.cfg NeverHistoryRollback
check DirectoryEvolution.tla cfg/directory-evolve-root-stage-witness.cfg NeverRootStaged
check DirectoryEvolution.tla cfg/directory-evolve-reactivation-witness.cfg NeverHistoryRollback
check DirectoryEvolution.tla cfg/directory-evolve-published-abort-witness.cfg NeverPublishedAbortStage
check DirectoryEvolution.tla cfg/directory-evolve-repeat-recovery-witness.cfg NeverRepeatedEvolutionRecovery
check DirectoryEvolution.tla cfg/directory-evolve-original-contents-witness.cfg NeverOriginalStageChildRestored
check DirectoryEvolution.tla cfg/directory-evolve-target-nonempty.cfg TargetInstallationEmpty
check DirectoryEvolution.tla cfg/directory-evolve-discard-foreign.cfg ForeignStageChildrenPreserved
# The same generalized hierarchy actions used by TLAPS also get finite
# discovery instances; these do not replace the operational durability seam.
check OwnedDirectoryMC.tla cfg/generalized/directory-empty.cfg
check OwnedDirectoryMC.tla cfg/generalized/directory-nested.cfg
check OwnedDirectoryMC.tla cfg/generalized/directory-missing-intent.cfg DirectorySafety
check OwnedDirectoryMC.tla cfg/generalized/directory-parent-first.cfg DirectorySafety
check OwnedDirectoryMC.tla cfg/generalized/directory-ancestor-substitution.cfg DirectorySafety
check RetainedDirectoryMC.tla cfg/generalized/directory-retained.cfg
check RetainedDirectoryMC.tla cfg/generalized/direct-migration-receipt.cfg
check IncarnationDomainMC.tla cfg/generalized/incarnation-exchange.cfg
check IncarnationDomainMC.tla cfg/generalized/incarnation-evolve-borrowed.cfg
check ProcessSupervision.tla ProcessSupervision.cfg
check ProcessSupervision.tla ProcessSupervision.ignore-deadline.cfg temporal:Terminates
check ProcessSupervision.tla ProcessSupervision.inherited-pipe.cfg temporal:Terminates
check ProcessSupervision.tla ProcessSupervision.reap-first.cfg PidAuthority
check ProcessSupervision.tla ProcessSupervision.cleanup-failure-witness.cfg NeverCleanupFailure
check ProcessSupervision.tla ProcessSupervision.inherited-witness.cfg NeverInherited
check ProcessSupervision.tla ProcessSupervision.linger-witness.cfg NeverLinger
# Manual tasks compose the same supervisor with persistent mutation ownership.
check ManualTaskLifetime.tla cfg/manual-task-lifetime.cfg
check ManualTaskLifetime.tla cfg/manual-task-overlap.cfg NoOverlap
check ManualTaskLifetime.tla cfg/manual-task-drop-unknown.cfg UnknownRetained
check ManualTaskLifetime.tla cfg/manual-task-unknown-witness.cfg NeverUnknown
check ManualTaskLifetime.tla cfg/manual-task-cleanup-unknown-witness.cfg NeverCleanupUnknown
check ManualTaskLifetime.tla cfg/manual-task-sequential-witness.cfg NeverSequential
check UpdatePublication.tla UpdatePublication.cfg
check UpdatePublication.tla UpdatePublication.stale-recheck.cfg NoDowngrade
check UpdatePublication.tla UpdatePublication.premature-rename.cfg AtomicReady
check UpdatePublication.tla UpdatePublication.rollback.cfg NoDowngrade
check UpdatePublication.tla UpdatePublication.post-failure-witness.cfg NeverPostFailure
check UpdatePublication.tla UpdatePublication.concurrent-witness.cfg NeverIndependentLocks
check UpdatePublication.tla UpdatePublication.recheck-witness.cfg NeverRecheckSkip
for surface in copy template merge link; do
    for mode in m0644 m0755 m0600; do
        check FileMode.tla "cfg/filemode-$surface-$mode.cfg"
    done
done
check FileMode.tla cfg/filemode-mutant-template-fixed0644.cfg ExecSurvivesDeploy
check FileMode.tla cfg/filemode-mutant-bytes-only.cfg ChmodIsDrift
check FileMode.tla cfg/filemode-mutant-prune.cfg PruneRespectsDrift
check FileMode.tla cfg/filemode-mutant-rollback.cfg RollbackIsExact

# 0044 protocol/policy contracts. Real-code bridges remain separate: a model
# checks its abstraction, not an arbitrary implementation or parser.
check UpdateSurvey.tla cfg/update-survey.cfg
check UpdateSurvey.tla cfg/update-survey-empty.cfg
check UpdateSurvey.tla cfg/update-survey-stop-first.cfg CompleteSurvey
check UpdateSurvey.tla cfg/update-survey-exit-conflation.cfg HonestExit
check UpdateSurvey.tla cfg/update-survey-publication.cfg NoPublication
check HttpRetry.tla cfg/http-retry.cfg
check HttpRetry.tla cfg/http-retry-terminal-replay.cfg NoTerminalReplay
check HttpRetry.tla cfg/http-retry-deadline-reset.cfg FixedDeadline
check HttpRetry.tla cfg/http-retry-attempt-overflow.cfg BoundedAttempts
check CredentialRouting.tla cfg/credential-routing.cfg
check CredentialRouting.tla cfg/credential-routing-base-authority.cfg TokensStayBound
check CredentialRouting.tla cfg/credential-routing-redirect-forwarding.cfg NoRedirectDisclosure
check CredentialRouting.tla cfg/credential-routing-repo-audience.cfg TokensStayBound
check MergeBoundary.tla cfg/merge-boundary.cfg
check MergeBoundary.tla cfg/merge-boundary-unclosed-marker.cfg ForeignTextPreserved
check MergeBoundary.tla cfg/merge-boundary-first-mode.cfg AllModeEvidenceRequired
check MergeBoundary.tla cfg/merge-boundary-first-prune.cfg PruneNeedsWholeEvidence

# B1 worker lease safety (plan/0051, Epic B 10.3): the positive two-client
# crash model with owner incarnations, monotone durable counters and
# quiescence receipts, plus four calibrated mutants — an early stop that
# ignores live leases, a crash that silently erases them, and retirements
# that ignore the owner incarnation or the epoch fence.
check WorkerLease.tla cfg/worker-lease.cfg
check WorkerLease.tla cfg/worker-lease-early-stop.cfg NoStopWithLiveLease
check WorkerLease.tla cfg/worker-lease-crash-wipes.cfg NoSilentLeaseVanish
check WorkerLease.tla cfg/worker-lease-retire-foreign-owner.cfg RetireRespectsOwner
check WorkerLease.tla cfg/worker-lease-retire-stale-epoch.cfg RetireRespectsEpoch

# Checked bridge sessions bind both operation and worker epochs. Exact replies
# stutter, while changed terminal payloads and foreign workers remain errors.
check BuildSession.tla cfg/build-session.cfg
check BuildSession.tla cfg/build-session-stale-identity.cfg MatchingExport
check BuildSession.tla cfg/build-session-stale-worker.cfg ForeignRefusal
check BuildSession.tla cfg/build-session-early-done.cfg MatchingExport
check BuildSession.tla cfg/build-session-conflicting-replay.cfg ConflictRefusal
check BuildSession.tla cfg/build-session-success-witness.cfg NeverCompletes
check BuildSession.tla cfg/build-session-cancel-witness.cfg NeverCancellation
check BuildSession.tla cfg/build-session-replay-witness.cfg NeverReplay

# M-V6: finite discovery instances of the shared, crash-unbounded protocol.
# The separate TLAPS catalog checks every generalized theorem dependency.
check UndoCellCheck.tla cfg/generalized/cell.cfg clean
check DestinationProduct.tla cfg/generalized/product-empty.cfg clean
check DestinationProduct.tla cfg/generalized/product-two.cfg clean
check SelectionLifecycle.tla cfg/generalized/selection-normal.cfg clean
check ObjectPublication.tla cfg/generalized/object-write.cfg clean
check ObjectPublication.tla cfg/generalized/object-observed.cfg clean
check ObjectPublication.tla cfg/generalized/object-file-sync.cfg PublishedObjectHasDurablePayload
check ObjectPublication.tla cfg/generalized/object-mode-order.cfg PublishedObjectHasDurablePayload
check ObjectPublication.tla cfg/generalized/object-parent-sync.cfg AuthorityHasDurableObject
check ObjectPublication.tla cfg/generalized/object-observed-file-sync.cfg AuthorityHasDurableObject
check NamespaceSealing.tla cfg/generalized/namespace-empty.cfg clean
check NamespaceSealing.tla cfg/generalized/namespace-one.cfg clean
check NamespaceSealing.tla cfg/generalized/namespace-deep.cfg clean
check NamespaceSealing.tla cfg/generalized/namespace-skip-existing.cfg AuthorityHasDurableNamespace
check NamespaceSealing.tla cfg/generalized/namespace-leaf-only.cfg AuthorityHasDurableNamespace
check GenerationPublication.tla cfg/generalized/generation-zero-space.cfg clean
check GenerationPublication.tla cfg/generalized/generation-fresh.cfg clean
check GenerationPublication.tla cfg/generalized/generation-retained-gap.cfg clean
check GenerationPublication.tla cfg/generalized/generation-legacy-floor.cfg clean
check GenerationPublication.tla cfg/generalized/generation-legacy-exhausted.cfg clean
check GenerationPublication.tla cfg/generalized/generation-missing-high-water.cfg NewPublicationsHaveStableHighWater
check GenerationPublication.tla cfg/generalized/generation-missing-file-sync.cfg PublishedGenerationHasDurableArtifacts
check GenerationPublication.tla cfg/generalized/generation-missing-parent-sync.cfg ReturnedPublicationIsStable
check PreparationPublication.tla cfg/generalized/preparation-empty.cfg clean 3g 2
check PreparationPublication.tla cfg/generalized/preparation-plan.cfg clean 3g 2
check PreparationPublication.tla cfg/generalized/preparation-plan-and-outcomes.cfg clean 3g 2
check PreparationPublication.tla cfg/generalized/preparation-file-sync.cfg PreparedDocumentsAreDurable 3g 2
check PreparationPublication.tla cfg/generalized/preparation-mode-order.cfg PreparedDocumentsAreDurable 3g 2
check PreparationPublication.tla cfg/generalized/preparation-parent-sync.cfg PreparedDocumentsAreDurable 3g 2
check PreparationPublication.tla cfg/generalized/preparation-incomplete-set.cfg PreparedDocumentsAreDurable 3g 2
check PreparationPublication.tla cfg/generalized/preparation-existing-namespace.cfg PreparedDocumentsAreDurable 3g 2
check PreparationPublication.tla cfg/generalized/preparation-leaf-namespace.cfg PreparedDocumentsAreDurable 3g 2
check ObjectNamespacePublication.tla cfg/generalized/reachable-new-object-nested-path.cfg clean
check ObjectNamespacePublication.tla cfg/generalized/reachable-observed-object-nested-path.cfg clean
check ObjectNamespacePublication.tla cfg/generalized/reachable-missing-object-file-sync.cfg ReturnedObjectIsReachableAndDurable
check ObjectNamespacePublication.tla cfg/generalized/reachable-observed-object-file-sync.cfg ReturnedObjectIsReachableAndDurable
check ObjectNamespacePublication.tla cfg/generalized/reachable-late-private-mode.cfg ReturnedObjectIsReachableAndDurable
check ObjectNamespacePublication.tla cfg/generalized/reachable-missing-object-parent-sync.cfg ReturnedObjectIsReachableAndDurable
check ObjectNamespacePublication.tla cfg/generalized/reachable-unsealed-existing-ancestors.cfg ReturnedObjectIsReachableAndDurable
check ObjectNamespacePublication.tla cfg/generalized/reachable-leaf-only-directory-sync.cfg ReturnedObjectIsReachableAndDurable
check JournalLifecycle.tla cfg/generalized/journal-empty.cfg clean 3g 2
check JournalLifecycle.tla cfg/generalized/journal-two-transactions.cfg clean 3g 2
check JournalLifecycle.tla cfg/generalized/journal-two-destinations.cfg clean 3g 2
check JournalLifecycle.tla cfg/generalized/journal-entry-removal-barrier.cfg MarkerCoversEntries 3g 2
check JournalLifecycle.tla cfg/generalized/journal-current-admission-barrier.cfg RecoveryEvidencePreserved 3g 2
check JournalLifecycle.tla cfg/generalized/journal-marker-publication-barrier.cfg MutationHasDurableMarker 3g 2
check JournalLifecycle.tla cfg/generalized/journal-generation-is-not-transaction.cfg ExactCommitIdentity 3g 2
check SelectionLifecycle.tla cfg/generalized/selection-generation-identity.cfg PredecessorCannotCommit
check PublicationSelection.tla cfg/generalized/publication-selection-empty.cfg clean 3g 2
check PublicationSelection.tla cfg/generalized/publication-selection-one-destination.cfg clean 3g 2
check PublicationSelection.tla cfg/generalized/publication-selection-unsealed-generation.cfg CurrentNamesDurableGeneration 3g 2
check ActivationLifecycleMC.tla cfg/generalized/activation-lifecycle-empty-intents.cfg clean 3g 2
check ActivationLifecycleMC.tla cfg/generalized/activation-lifecycle-one-intent.cfg clean 3g 2
check ActivationLifecycleMC.tla cfg/generalized/activation-lifecycle-mixed-legacy.cfg clean 3g 2
check ActivationLifecycleMC.tla cfg/generalized/activation-lifecycle-one-destination.cfg clean 3g 2
check PreparedActivationMC.tla cfg/generalized/prepared-lifecycle-no-hooks.cfg clean 3g 2
check PreparedActivationMC.tla cfg/generalized/prepared-lifecycle-prepared-hook.cfg clean 3g 2
check PreparedActivationMC.tla cfg/generalized/prepared-lifecycle-destination-hook.cfg clean 3g 2
check PreparedActivationMC.tla cfg/generalized/prepared-lifecycle-unsealed-initial-file.cfg EveryPlanHasDurablePreparation 3g 2
check PreparedActivationMC.tla cfg/generalized/prepared-lifecycle-unsealed-initial-name.cfg EveryPlanHasDurablePreparation 3g 2
check PreparedActivationMC.tla cfg/generalized/prepared-lifecycle-missing-initial-outcome.cfg EveryPlanHasDurablePreparation 3g 2
check PreparedActivationMC.tla cfg/generalized/prepared-lifecycle-unsealed-instance-namespace.cfg EveryPlanHasDurablePreparation 3g 2
check LifecycleRetentionMC.tla cfg/generalized/retention-retained-history.cfg clean 3g 2
check LifecycleRetentionMC.tla cfg/generalized/retention-legacy-floor.cfg clean 3g 2
check LifecycleRetentionMC.tla cfg/generalized/retention-missing-prune-barrier.cfg NoProtectedRootCollection 3g 2
check LifecycleRetentionMC.tla cfg/generalized/retention-missing-allocation-floor.cfg AllocationHistoryCovered 3g 2
check LifecycleRetentionMC.tla cfg/generalized/retention-ignored-pending-work.cfg NoProtectedRootCollection 3g 2
check LifecycleRetentionMC.tla cfg/generalized/retention-fresh-journal-and-activation.cfg clean 3g 2
check ActivationInvariant.tla cfg/generalized/activation-empty.cfg clean
check ActivationInvariant.tla cfg/generalized/activation-heterogeneous.cfg clean
