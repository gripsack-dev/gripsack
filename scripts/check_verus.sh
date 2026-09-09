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
# classify + its contract obligations; if the kernel grows, grow this.
MIN_OBLIGATIONS=3

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

echo "== calibration: a semantic mutant must fail its postcondition"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
# a standalone copy — the kernel crate has no workspace dependencies
mkdir -p "$tmp/mutant/src"
cp "$CRATE/src/lib.rs" "$tmp/mutant/src/lib.rs"
sed -e 's/name = "gripsack-policy"/name = "gripsack-policy-mutant"/' \
    -e 's/^version\.workspace = true/version = "0.0.0"/' \
    -e 's/^edition\.workspace = true/edition = "2024"/' \
    -e '/^license\.workspace/d' -e '/^repository\.workspace/d' \
    "$CRATE/Cargo.toml" > "$tmp/mutant/Cargo.toml"
# the mutant: ambiguity misread as commitment (a crashed roll-forward
# falsely committing is the 0.22 bug class this classifier exists to kill)
sed -i 's/(Some(_), _) => Classification::Ambiguous,/(Some(_), _) => Classification::Committed,/' \
    "$tmp/mutant/src/lib.rs"
if ! grep -q '(Some(_), _) => Classification::Committed,' "$tmp/mutant/src/lib.rs"; then
    echo "FAIL: the calibration mutant did not apply — the kernel changed shape; recalibrate"
    exit 1
fi
if out2="$(cd "$tmp/mutant" && cargo verus verify 2>&1)"; then
    echo "FAIL: the mutant VERIFIED — the proof does not see the classification table"
    exit 1
fi
echo "$out2" | grep -m3 "postcondition not satisfied" || {
    echo "$out2" | tail -20
    echo "FAIL: the mutant failed without a named postcondition — unrecognised failure, not calibration evidence"
    exit 1
}
echo "calibration: mutant rejected on its postcondition"
echo "verify gate: OK"
