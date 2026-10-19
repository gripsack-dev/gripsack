#!/bin/sh
# B0-01 qualification driver (handover Epic B, Linux lane).
#
# Brings up a pinned disposable buildkitd, runs the Go-LLB bridge
# probes, tears the worker down between phases (retained-output and
# clean-reproduction requirements), verifies OCI layouts with an
# independent python checker, and records versions/configuration and
# measurements into results/.
#
# Mac VM qualification is outside the active platform scope
# (PLATFORM-LINUX-X64-2026-10-09), not a missing Linux probe.
#
# Usage: sh probes.sh            (from verification/buildkit-qualification)
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
BRIDGE=$HERE/bridge/bridge-bin
RESULTS=$HERE/results
OWNER=$(python3 -c 'import uuid; print(uuid.uuid4().hex)')
OWNER_LABEL=dev.gripsack.local-ci.b0-owner
WORKER=
WORKER_ID=
CACHE=
CIDFILE=
ROUND=0
OWNED_TAG=
OWNED_IMAGE=
worker_destroy() {
  if [ -n "$CIDFILE" ] && [ -f "$CIDFILE" ]; then
    WORKER_ID=$(cat "$CIDFILE")
  fi
  if [ -n "$WORKER_ID" ]; then
    actual=$(docker inspect -f "{{index .Config.Labels \"$OWNER_LABEL\"}}" "$WORKER_ID") || return 1
    [ "$actual" = "$OWNER" ] || { echo "FAIL: foreign B0 worker" >&2; return 1; }
    docker rm -f -v "$WORKER_ID" >/dev/null || return 1
    WORKER_ID=
    CIDFILE=
  fi
  if [ -n "$CACHE" ]; then
    actual=$(docker volume inspect -f "{{index .Labels \"$OWNER_LABEL\"}}" "$CACHE") || return 1
    [ "$actual" = "$OWNER" ] || { echo "FAIL: foreign B0 cache" >&2; return 1; }
    docker volume rm "$CACHE" >/dev/null || return 1
    CACHE=
  fi
}
cleanup() {
  worker_destroy || return 1
  if [ -n "$OWNED_IMAGE" ]; then
    actual=$(docker image inspect -f '{{.Id}}' "$OWNED_TAG") || return 1
    [ "$actual" = "$OWNED_IMAGE" ] || { echo "FAIL: B0 image tag changed" >&2; return 1; }
    docker rmi "$OWNED_TAG" >/dev/null || return 1
    OWNED_IMAGE=
    OWNED_TAG=
  fi
}
trap 'status=$?; trap - EXIT; cleanup || status=1; exit "$status"' EXIT

REPO=$(cd "$HERE/../.." && pwd)
. "$HERE/pins.env"

# Fresh per-invocation worker/cache names; never delete a pre-existing resource.
# Bridge execution uses docker exec, so a containerized operator needs no
# assumption that its own loopback is the Docker daemon's published-port host.

mkdir -p "$RESULTS"
ENV_OUT="$RESULTS/environment.txt"
{
  echo "date: $(date -u +%FT%TZ)"
  echo "kernel: $(uname -srm)"
  echo "docker: $(docker info --format '{{.ServerVersion}} {{.OSType}}/{{.Architecture}}')"
  echo "buildkit-image: $BUILDKIT_IMAGE"
  echo "golang-build-image: $GOLANG_IMAGE"
  echo "gcc-in-graph: $GCC_IMAGE"
  echo "alpine-in-graph: $ALPINE_IMAGE"
} > "$ENV_OUT"

# run_bridge LOG ARGS...: a failed probe must fail the run — plain sh
# has no pipefail, so bridge output is redirected and checked, then shown.
run_bridge() {
  log=$1; shift
  if "$@" > "$log" 2>&1; then
    cat "$log"
  else
    rc=$?
    cat "$log"
    echo "FAIL: bridge exited $rc (log: $log)" >&2
    exit "$rc"
  fi
}

worker_start() {
  ROUND=$((ROUND + 1))
  WORKER="gripsack-b0-$OWNER-$ROUND"
  cache_name="gripsack-b0-cache-$OWNER-$ROUND"
  if docker container inspect "$WORKER" >/dev/null 2>&1 \
      || docker volume inspect "$cache_name" >/dev/null 2>&1; then
    echo "FAIL: B0 resource name already exists; refusing adoption" >&2; exit 1
  fi
  docker info --format '{{.ServerVersion}}' >/dev/null
  docker volume create --label "$OWNER_LABEL=$OWNER" "$cache_name" >/dev/null
  CACHE=$cache_name
  CIDFILE="$RESULTS/worker-$ROUND.cid"
  docker run -d --privileged --platform linux/amd64 --name "$WORKER" \
    --cidfile "$CIDFILE" --label "$OWNER_LABEL=$OWNER" \
    -v "$CACHE":/var/lib/buildkit \
    -v "$RESULTS":"$RESULTS" \
    -v "$HERE/pins.env":"$HERE/pins.env":ro \
    -v "$BRIDGE":/usr/local/bin/gripsack-b0-bridge:ro \
    "$BUILDKIT_IMAGE" --addr tcp://0.0.0.0:12341 >> "$ENV_OUT"
  WORKER_ID=$(cat "$CIDFILE")
  # health: workers must answer over the TCP address before any probe
  # claims the lane; a container that never becomes healthy fails the run
  i=0
  until docker exec "$WORKER_ID" buildctl --addr tcp://127.0.0.1:12341 debug workers >/dev/null 2>&1; do
    i=$((i + 1))
    if [ $i -ge 45 ]; then
      echo "FAIL: $WORKER never became healthy" >&2
      docker logs "$WORKER_ID" >&2
      exit 1
    fi
    sleep 1
  done
  # A working daemon without an observed worker list and actual version
  # is not a qualified worker; plain-sh tee must not hide either failure.
  run_bridge "$RESULTS/worker-health.log" docker exec "$WORKER_ID" \
    buildctl --addr tcp://127.0.0.1:12341 debug workers
  cat "$RESULTS/worker-health.log" >> "$ENV_OUT"
  run_bridge "$RESULTS/buildkit-version.log" docker exec "$WORKER_ID" buildkitd --version
  cat "$RESULTS/buildkit-version.log" >> "$ENV_OUT"
}


# --- probe 1: baseline preservation --------------------------------
# A word in a comment or an unsupported-input test is not a dependency.
# Check the resolved Rust package graph instead; the full offline
# product flow is exercised by the separate e2e container gate.
baseline() {
  echo "--- probe1: baseline preservation"
  python3 - "$REPO/Cargo.lock" <<'PY' || return 1
import sys
import tomllib

with open(sys.argv[1], "rb") as lockfile:
    packages = tomllib.load(lockfile)["package"]
builder = sorted(
    pkg["name"] for pkg in packages
    if pkg["name"] != "gripsack-buildkit"  # Gripsack's OWN adapter crate (plan/0051):
                                           # base64/serde/thiserror/sha2 only — its
                                           # presence must not read as an upstream
                                           # builder dependency.
    and any(name in pkg["name"].lower()
            for name in ("buildkit", "moby", "docker", "containerd", "bollard"))
)
if builder:
    sys.exit(f"FAIL: native Rust lock gained builder dependencies: {builder}")
# The owned adapter itself must stay builder-free: if it ever grows an
# upstream builder dependency, that is exactly the baseline violation
# this probe exists to catch.
adapter = [pkg for pkg in packages if pkg["name"] == "gripsack-buildkit"]
if adapter:
    upstream = [dep for dep in adapter[0].get("dependencies", [])
                if any(name in dep.lower()
                       for name in ("buildkit", "moby", "docker", "containerd", "bollard"))]
    if upstream:
        sys.exit(f"FAIL: gripsack-buildkit gained upstream builder dependencies: {upstream}")
print("native Rust package graph: no builder dependencies")
PY
  docker info --format '{{.ServerVersion}} {{.OSType}}/{{.Architecture}}' || return 1
  if [ -x "$REPO/target/debug/grip" ]; then
    if ldd "$REPO/target/debug/grip" 2>/dev/null | grep -qi "docker\|containerd"; then
      echo "FAIL: grip links container runtime libraries" >&2
      return 1
    fi
    echo "grip binary: no container-runtime library linkage"
  else
    echo "note: target/debug/grip not built; binary linkage not assessed here"
  fi
}
run_bridge "$RESULTS/probe1-baseline.txt" baseline

# --- probes 2, 3, 5 against worker #1 ------------------------------
worker_start
START=$(date +%s)
run_bridge "$RESULTS/probe2.log" docker exec "$WORKER_ID" /usr/local/bin/gripsack-b0-bridge -addr tcp://127.0.0.1:12341 -out "$RESULTS" -pins "$HERE/pins.env" -probe graph
run_bridge "$RESULTS/probe5.log" docker exec "$WORKER_ID" /usr/local/bin/gripsack-b0-bridge -addr tcp://127.0.0.1:12341 -out "$RESULTS" -pins "$HERE/pins.env" -probe policy
run_bridge "$RESULTS/probe3.log" docker exec "$WORKER_ID" /usr/local/bin/gripsack-b0-bridge -addr tcp://127.0.0.1:12341 -out "$RESULTS" -pins "$HERE/pins.env" -probe executable
WORKER1_SECONDS=$(( $(date +%s) - START ))

# --- probe 3 teardown: retained output survives worker+cache death --
worker_destroy
OUT=$("$HERE/results/executable/hello")
echo "probe3 retained run after worker+cache removal: $OUT" | tee -a "$RESULTS/probe3.log"
[ "$OUT" = "gripsack-b0-static-hello" ]

# --- probe 4: two FRESH workers, clean reproduction ------------------
worker_start
S1=$(date +%s)
run_bridge "$RESULTS/probe4-a.log" docker exec "$WORKER_ID" /usr/local/bin/gripsack-b0-bridge -addr tcp://127.0.0.1:12341 -out "$RESULTS/oci-a" -pins "$HERE/pins.env" -probe oci
A_SECONDS=$(( $(date +%s) - S1 ))
mv "$RESULTS/oci-a/oci-layout.tar" "$RESULTS/oci-1.tar"
worker_destroy

worker_start
S2=$(date +%s)
run_bridge "$RESULTS/probe4-b.log" docker exec "$WORKER_ID" /usr/local/bin/gripsack-b0-bridge -addr tcp://127.0.0.1:12341 -out "$RESULTS/oci-b" -pins "$HERE/pins.env" -probe oci
B_SECONDS=$(( $(date +%s) - S2 ))
mv "$RESULTS/oci-b/oci-layout.tar" "$RESULTS/oci-2.tar"
worker_destroy

echo "worker1-total-seconds: $WORKER1_SECONDS" >> "$ENV_OUT"
echo "oci-clean-build-a-seconds: $A_SECONDS" >> "$ENV_OUT"
echo "oci-clean-build-b-seconds: $B_SECONDS" >> "$ENV_OUT"

# Independent verification and clean-worker reproduction MUST complete
# before a portable Docker archive is made from the checked OCI config
# and exact uncompressed layer bytes. Older Docker image stores cannot
# load a pure OCI-layout tar (CI's Docker 28 rejected blobs/json).
run_bridge "$RESULTS/probe4-verify.json" python3 "$HERE/verify_oci.py" \
  "$RESULTS/oci-1.tar" "$RESULTS/oci-2.tar" "$RESULTS/docker-compat.tar"

IMAGE_TAG=$(python3 - "$RESULTS/probe4-verify.json" <<'PY'
import json
import sys
with open(sys.argv[1]) as report:
    verified = json.load(report)
if verified["first"]["config-digest"] != verified["docker-archive"]["config-digest"]:
    sys.exit("OCI config identity changed in portable Docker archive")
print(verified["docker-archive"]["tag"])
PY
)
# A deterministic config-derived tag isolates this fixture. Never
# overwrite/remove an image which already belonged to this Docker host.
if docker image inspect "$IMAGE_TAG" >/dev/null 2>&1; then
  echo "FAIL: Docker already has the B0 fixture tag $IMAGE_TAG" >&2
  exit 1
fi

# Docker may re-encode config metadata, so its image ID need not be
# the OCI config blob SHA. Inspect actual loaded layers/platform/runtime
# fields independently and compare with the already verified OCI report.
if docker load -i "$RESULTS/docker-compat.tar" > "$RESULTS/probe4-load.txt" 2>&1; then
  if ! docker image inspect "$IMAGE_TAG" > "$RESULTS/probe4-image-inspect.json" 2>&1; then
    cat "$RESULTS/probe4-load.txt" >&2
    cat "$RESULTS/probe4-image-inspect.json" >&2
    echo "FAIL: Docker load did not publish the verified image tag" | tee "$RESULTS/probe4-runtime.txt"
    exit 1
  fi
  OWNED_TAG=$IMAGE_TAG
  OWNED_IMAGE=$(docker image inspect -f '{{.Id}}' "$OWNED_TAG")
  run_bridge "$RESULTS/probe4-runtime-check.log" python3 "$HERE/check_loaded.py" \
    "$RESULTS/probe4-verify.json" "$RESULTS/probe4-image-inspect.json"
  if docker run --rm "$IMAGE_TAG" cat /hello.txt > "$RESULTS/probe4-run.txt" 2>&1 \
      && [ "$(cat "$RESULTS/probe4-run.txt")" = "gripsack b0 oci fixture" ]; then
    cat "$RESULTS/probe4-run.txt"
    cleanup
    echo "separate-runtime: Docker loaded checked OCI layers/config via portable archive and ran image" | tee "$RESULTS/probe4-runtime.txt"
  else
    if [ -f "$RESULTS/probe4-run.txt" ]; then
      cat "$RESULTS/probe4-run.txt" >&2
    fi
    echo "FAIL: Docker image load succeeded but independent run/content failed" | tee "$RESULTS/probe4-runtime.txt"
    exit 1
  fi
else
  cat "$RESULTS/probe4-load.txt" >&2
  echo "FAIL: Docker load rejected the verified OCI-backed portable archive" | tee "$RESULTS/probe4-runtime.txt"
  exit 1
fi

echo "=== B0-01 Linux lane probes complete; results in $RESULTS"
echo "B0_LINUX_QUALIFICATION=6"
