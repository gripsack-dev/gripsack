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
    log="$work/$(basename "$config").log"
    status=0
    (cd "$spec" && java -Xmx1g -cp "$jar" tlc2.TLC -cleanup -workers 1 \
        -metadir "$work/states" -config "$config" "$module") >"$log" 2>&1 || status=$?
    case "$expected" in
        clean)
            if [ "$status" -ne 0 ] || ! grep -q 'Model checking completed. No error' "$log"; then
                cat "$log"; echo "FAIL: $config" >&2; exit 1
            fi
            ;;
        temporal:*)
            property=${expected#temporal:}
            # The negative must name exactly one temporal property, and it
            # must be the intended one: a temporal failure can then never
            # calibrate a different liveness claim by accident.
            props=$(grep -c '^[[:space:]]*PROPERT' "$cfg")
            named=$(grep '^[[:space:]]*PROPERT' "$cfg" | tr -s ' \t' ' ' | tr -d '\r' \
                | sed 's/^ *//')
            if [ "$props" -ne 1 ] || [ "$named" != "PROPERTY $property" ] \
                || ! grep -Eq 'Temporal propert(y|ies) .* (was|were) violated' "$log"; then
                cat "$log"; echo "FAIL: $config did not solely violate $property" >&2; exit 1
            fi
            ;;
        *)
            # The negative must actually check the named invariant, so the
            # reported violation is the intended calibration.
            if ! tr -s ' \t' '\n' <"$cfg" | grep -qx "$expected" \
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
    check MultiDestination.tla "cfg/repeated-$mode.cfg"
done
check Activation.tla cfg/activation.cfg
check RepeatedActivation.tla cfg/repeated-activation.cfg
check RepeatedActivation.tla cfg/repeated-activation-repeated-generation.cfg
check MultiDestination.tla cfg/repeated-transaction-premature-cleanup.cfg RestoreBeforeCleanup
check RepeatedActivation.tla cfg/repeated-activation-lost-pending.cfg NoSilentSkip
check ProcessSupervision.tla ProcessSupervision.cfg
check ProcessSupervision.tla ProcessSupervision.ignore-deadline.cfg temporal:Terminates
check ProcessSupervision.tla ProcessSupervision.inherited-pipe.cfg temporal:Terminates
check ProcessSupervision.tla ProcessSupervision.reap-first.cfg PidAuthority
check ProcessSupervision.tla ProcessSupervision.cleanup-failure-witness.cfg NeverCleanupFailure
check ProcessSupervision.tla ProcessSupervision.inherited-witness.cfg NeverInherited
check ProcessSupervision.tla ProcessSupervision.linger-witness.cfg NeverLinger
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
check MergeBoundary.tla cfg/merge-boundary.cfg
check MergeBoundary.tla cfg/merge-boundary-unclosed-marker.cfg ForeignTextPreserved
check MergeBoundary.tla cfg/merge-boundary-first-mode.cfg AllModeEvidenceRequired
check MergeBoundary.tla cfg/merge-boundary-first-prune.cfg PruneNeedsWholeEvidence
