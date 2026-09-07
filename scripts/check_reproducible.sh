#!/bin/sh
# Two-clean-build reproducibility comparison (plan/0042 F).
#
# Holds every build input fixed — same source tree, same Cargo.lock,
# same built image (BOTH runs are pinned to the captured immutable
# image ID via a temporary compose override, pull_policy never — never
# a mutable service tag), same container path — and compiles the
# release target twice from scratch (a fresh container with a fresh
# CARGO_TARGET_DIR per build; the compose `repro` service guarantees
# the clean target dir).
#
# Output goes to a UNIQUE run directory under the given parent
# (default ./repro): repro/run-<utcstamp>-<pid>/. Nothing preexisting
# is ever removed or overwritten. The script exports REPRO_OUTPUT (the
# absolute run dir), which the compose service binds at /dist.
#
# Emits, inside the run dir:
#   repro-a/  grip, grip.sha256, toolchain.txt
#   repro-b/  grip, grip.sha256, toolchain.txt
#   build-a.log, build-b.log
#   report.md, report.json
#
# The gate fails (exit 1) unless both builds recorded the identical
# toolchain AND the stripped binaries are byte-identical. It also
# fails closed if the built image's identity cannot be captured —
# "same image for both builds" is proven, never assumed.
#
# What this is NOT (plan/0042 F): not a cross-time claim (rebuilding
# after a deliberate toolchain/digest bump may differ), not a
# cross-platform claim (musl vs darwin binaries are different targets),
# and not an external audit. It records one controlled experiment.
#
# Usage: scripts/check_reproducible.sh [output-parent]   (default: ./repro)
set -eu
cd "$(dirname "$0")/.."
parent=${1:-repro}

command -v docker >/dev/null 2>&1 || { echo "docker is required" >&2; exit 1; }

# Fixed inputs, recorded from the host side.
git_rev="not-a-git-checkout"
git_dirty="n/a"
if command -v git >/dev/null 2>&1 && git rev-parse HEAD >/dev/null 2>&1; then
    git_rev=$(git rev-parse HEAD)
    if [ -n "$(git status --porcelain)" ]; then git_dirty=yes; else git_dirty=no; fi
fi
lock_sha=$(sha256sum Cargo.lock | cut -d' ' -f1)
# base image fingerprint DERIVED from the Dockerfile, not hardcoded here
base_digest=$(sed -n 's/^FROM rust:alpine@\(sha256:[0-9a-f]\{64\}\) AS builder$/\1/p' Dockerfile)
if [ -z "$base_digest" ]; then
    echo "FAIL: could not parse the rust:alpine digest pin from Dockerfile" >&2
    exit 1
fi
date_utc=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

# Unique run directory — created, never removed; prior runs (even
# root-owned container output) are left untouched.
mkdir -p "$parent"
out=$(cd "$parent" && pwd)/run-$(date -u '+%Y%m%dT%H%M%SZ')-$$
if [ -e "$out" ]; then
    echo "FAIL: run dir already exists: $out" >&2
    exit 1
fi
mkdir "$out"
# The compose service binds exactly this absolute directory at /dist.
REPRO_OUTPUT=$out
export REPRO_OUTPUT

# One image build; the captured image ID then pins BOTH runs.
docker compose build repro

image_ref=$(docker compose images -q repro 2>/dev/null || true)
if [ -z "$image_ref" ]; then
    image_ref=$(docker image inspect -f '{{.Id}}' "$(basename "$PWD")-repro" 2>/dev/null || true)
fi
image_id=$(docker image inspect -f '{{.Id}}' "$image_ref" 2>/dev/null || true)
case "$image_id" in
    sha256:*) ;; # full immutable content-addressed ID
    *) echo "FAIL: could not capture the built image's identity (got: '$image_id'); refusing to claim same-image builds" >&2
       exit 1 ;;
esac
image_created=$(docker image inspect -f '{{.Created}}' "$image_id")

# Temporary compose override pinning the service to the captured image
# ID: both runs compile inside byte-for-byte that image; pull_policy
# never keeps a registry pull from silently substituting a tag.
override=$(mktemp)
trap 'rm -f "$override"' EXIT HUP INT TERM
cat > "$override" <<EOF
services:
  repro:
    image: $image_id
    pull_policy: never
EOF

for id in a b; do
    echo "== clean build $id (fresh container, fresh target dir, image $image_id)"
    # No pipeline: docker's own exit status IS the gate's status; the
    # log is echoed only after a failure, in full.
    if ! docker compose -f docker-compose.yml -f "$override" run --rm \
            -e BUILD_ID="$id" repro > "$out/build-$id.log" 2>&1; then
        cat "$out/build-$id.log"
        echo "FAIL: clean build $id exited nonzero" >&2
        exit 1
    fi
done

sha_a=$(cut -d' ' -f1 "$out/repro-a/grip.sha256")
sha_b=$(cut -d' ' -f1 "$out/repro-b/grip.sha256")

if cmp -s "$out/repro-a/grip" "$out/repro-b/grip"; then matched=yes; else matched=no; fi
if cmp -s "$out/repro-a/toolchain.txt" "$out/repro-b/toolchain.txt"; then toolchain_identical=yes; else toolchain_identical=no; fi

python3 - "$out" "$git_rev" "$git_dirty" "$lock_sha" "$base_digest" "$image_id" \
        "$image_created" "$date_utc" "$sha_a" "$sha_b" "$matched" \
        "$toolchain_identical" <<'PYEOF'
import json, sys
(out, git_rev, git_dirty, lock_sha, base_digest, image_id, image_created,
 date_utc, sha_a, sha_b, matched, toolchain_identical) = sys.argv[1:]
report = {
    "experiment": "two-clean-build release-target comparison (plan/0042 F)",
    "date": date_utc,
    "target": "x86_64-unknown-linux-musl (rust:alpine builder, release profile)",
    "held_fixed": {
        "source": {"git": git_rev, "worktree_dirty": git_dirty},
        "cargo_lock_sha256": lock_sha,
        "builder_image": {
            "base_pin": f"rust:alpine@{base_digest} (parsed from Dockerfile)",
            "built_image_id": image_id,
            "image_created": image_created,
            "same_image_for_both_builds": {
                "proven_by": "compose override pins image=<built_image_id>, pull_policy=never for BOTH runs",
                "value": True,
            },
        },
        "container_workdir": "/app (identical for both builds)",
        "target_dir": "fresh per build (CARGO_TARGET_DIR=/tmp/repro-target in a fresh container)",
        "toolchain": "see toolchain.txt (rustc/cargo/cargo-auditable; asserted identical between builds)",
        "profile": "workspace [profile.release], unmodified between builds",
    },
    "not_claimed": [
        "cross-time reproducibility (a later toolchain/digest bump may produce different bytes)",
        "cross-platform reproducibility (darwin targets are separate builds)",
        "external audit",
    ],
    "builds": {"a": {"sha256": sha_a}, "b": {"sha256": sha_b}},
    "binaries_byte_identical": matched == "yes",
    "toolchain_identical_between_builds": toolchain_identical == "yes",
}
with open(f"{out}/report.json", "w") as f:
    json.dump(report, f, indent=2)
    f.write("\n")
PYEOF

cat > "$out/report.md" <<EOF
# Two-clean-build comparison (plan/0042 F)

Run directory: $out
Date: $date_utc
Target: release (musl static, cargo auditable build --release --locked -p gripsack)

## Inputs held fixed

- Source: git $git_rev (dirty worktree: $git_dirty)
- Cargo.lock sha256: $lock_sha
- Builder base pin (parsed from Dockerfile): rust:alpine@$base_digest
- Built image: $image_id (created: $image_created)
  - both runs pinned to exactly this image ID via compose override
    (image=<id>, pull_policy never) — same-image is proven, not assumed
- Container workdir: /app (same path both builds)
- Target dir: fresh per build (fresh container, CARGO_TARGET_DIR=/tmp/repro-target)
- Toolchain: identical between builds ($toolchain_identical) — see toolchain.txt
- Profile: workspace [profile.release], unmodified between builds

## Result

- build a sha256: $sha_a
- build b sha256: $sha_b
- binaries byte-identical: $matched

## Not claimed

Pinning a compiler does not imply cross-time reproducibility (a deliberate
toolchain bump changes inputs by design), cross-platform identity (darwin
targets are separate builds), or an external audit.
EOF

cat "$out/report.md"

if [ "$matched" != yes ] || [ "$toolchain_identical" != yes ]; then
    echo "FAIL: two-clean-build comparison did not hold (matched=$matched toolchain_identical=$toolchain_identical); artifacts: $out" >&2
    exit 1
fi
echo "reproducible: two clean builds matched (artifacts: $out)"
