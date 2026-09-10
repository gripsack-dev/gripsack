#!/bin/sh
# The verification gate (plan/0046): prove the policy kernels, then
# prove the proof. Two halves, both required:
#
#   1. positive — `cargo verus verify -p gripsack-policy --locked`
#      must succeed with 0 errors and at least the expected obligation
#      count (zero/subset verification is not success).
#   2. calibration — a seeded semantic mutant (a crashed roll-forward
#      misread as committed: the 0.22 bug class) must FAIL its
#      postcondition. A verification failure for the intended contract
#      is required; a crash, parse error or missing solver is not.
#
# Toolchain (all three pins move together; updates are deliberate):
#   Verus release 0.2026.09.06.8dea4a2  (provides cargo-verus + verus)
#   Rust 1.98.0                          (the repo's pinned toolchain)
#   Z3 4.16.0                            (VERUS_Z3_PATH)
# vstd is pinned in the crate's manifest to the crates.io build
# published from the same Verus commit (=0.0.0-2026-09-06-0133).
set -eu

CRATE=crates/gripsack-policy
# classify + plan_copy + plan_link + the retention kernels (admission,
# prune, delete, membership helpers), the merge splice kernel, the
# graph closure kernels and the scheduler transition system, with
# their contracts; if the kernel set grows, grow this floor.
MIN_OBLIGATIONS=50

# verification results are cached by cargo — the gate always runs a
# CLEAN verification (a stale cache is not evidence)
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export CARGO_TARGET_DIR="$tmp/target"

echo "== positive: cargo verus verify -p gripsack-policy --locked"
if ! out="$(cargo verus verify -p gripsack-policy --locked 2>&1)"; then
    echo "$out" | tail -20
    echo "FAIL: verification errored"
    exit 1
fi
line="$(echo "$out" | grep -o 'verification results:: [0-9]* verified, [0-9]* errors' | tail -1)"
if [ -z "$line" ]; then
    echo "$out" | tail -10
    echo "FAIL: no 'verification results' line — nothing was verified"
    exit 1
fi
verified="$(echo "$line" | sed 's/.*:: \([0-9]*\) verified.*/\1/')"
errors="$(echo "$line" | sed 's/.*verified, \([0-9]*\) errors/\1/')"
if [ "$errors" != "0" ]; then
    echo "FAIL: $errors verification errors"
    exit 1
fi
if [ "$verified" -lt "$MIN_OBLIGATIONS" ]; then
    echo "FAIL: only $verified obligations checked (want >= $MIN_OBLIGATIONS) — subset verification reported as success is not success"
    exit 1
fi
echo "positive: $line"

echo "== calibration: semantic mutants must fail their postconditions"

# One calibration mutant per kernel family. Each replaces an exact
# source string; the replacement must APPLY (a stale pattern fails the
# harness, not the proof) and the mutated crate must fail verification
# with a named unsatisfied postcondition — a crash, parse error or
# missing solver is not calibration evidence.
run_mutant() {
    name="$1"; file="$2"; from="$3"; to="$4"
    rm -rf "$tmp/mutant"
    mkdir -p "$tmp/mutant"
    export CARGO_TARGET_DIR="$tmp/target-$name"
    cp -r "$CRATE/src" "$tmp/mutant/src"
    sed -e 's/name = "gripsack-policy"/name = "gripsack-policy-mutant"/' \
        -e 's/^version\.workspace = true/version = "0.0.0"/' \
        -e 's/^edition\.workspace = true/edition = "2024"/' \
        -e '/^license\.workspace/d' -e '/^repository\.workspace/d' \
        "$CRATE/Cargo.toml" > "$tmp/mutant/Cargo.toml"
    target="$tmp/mutant/src/$file"
    if ! grep -qF "$from" "$target"; then
        echo "FAIL: mutant pattern for $name no longer matches $file — the kernel changed shape; recalibrate"
        exit 1
    fi
    sed -i "s|$(printf '%s' "$from" | sed 's/[&|\\[]/\\&/g')|$(printf '%s' "$to" | sed 's/[&|\\]/\\&/g')|" "$target"
    if out2="$(cd "$tmp/mutant" && cargo verus verify 2>&1)"; then
        echo "FAIL: the $name mutant VERIFIED — the proof does not see the contract"
        exit 1
    fi
    echo "$out2" | grep -m1 "not satisfied" >/dev/null || {
        echo "$out2" | tail -20
        echo "FAIL: the $name mutant failed without a named unsatisfied contract — unrecognised failure, not calibration evidence"
        exit 1
    }
    echo "calibration: $name mutant rejected on its contract"
}

# classifier: ambiguity misread as commitment (the 0.22 bug class)
run_mutant "classifier" lib.rs \
    '(Some(_), _) => Classification::Ambiguous,' \
    '(Some(_), _) => Classification::Committed,'

# merge splice: the final foreign tail dropped from the output (the
# "merge silently ate my config" class)
run_mutant "merge-splice" merge.rs \
    '    out.extend_from_slice(&text[cursor..]);' \
    ''

# graph closure: a visited node never recorded in the result — the "a
# reachable module missing from the build closure" class — must fail
# the result-membership contract
run_mutant "graph-closure" graph.rs \
    '                    result.push(target);' \
    ''
# scheduler: starting work after the failure latch — the "a failed
# dependency authorized its consumer" class — must fail the named
# postcondition (old(self).failed ==> result.is_none())
run_mutant "scheduler-latch" schedule.rs \
    '        if self.failed || self.head >= self.ready.len() {' \
    '        if self.head >= self.ready.len() {'

echo "verify gate: OK"
