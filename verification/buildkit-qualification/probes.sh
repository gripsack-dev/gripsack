#!/bin/sh
# B0-01 qualification driver (handover Epic B, Linux lane).
#
# Brings up a pinned disposable buildkitd, runs the Go-LLB bridge
# probes, tears the worker down between phases (retained-output and
# clean-reproduction requirements), verifies OCI layouts with an
# independent python checker, and records versions/configuration and
# measurements into results/.
#
# Probe 6 (Mac VM qualification) is NOT here: no Mac hardware on this
# workstation; that lane stays blocked in the delivery ledger.
#
# Usage: sh probes.sh            (from verification/buildkit-qualification)
set -eu
trap "docker rm -f -v b0-buildkit >/dev/null 2>&1 || true" EXIT

HERE=$(cd "$(dirname "$0")" && pwd)
BRIDGE=$HERE/bridge/bridge-bin
RESULTS=$HERE/results
PORT=127.0.0.1:12341
WORKER=b0-buildkit
CACHE=b0-buildkit-cache

REPO=$(cd "$HERE/../.." && pwd)
. "$HERE/pins.env"

# Cold-start guarantee: no stale worker or cache volume from an earlier
# aborted run may warm this one (the EXIT trap covers failure paths).
docker rm -f -v "$WORKER" >/dev/null 2>&1 || true
docker volume rm "$CACHE" >/dev/null 2>&1 || true

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
  docker run -d --privileged --platform linux/amd64 --name "$WORKER" \
    -v "$CACHE":/var/lib/buildkit \
    -p "$PORT:12341" \
    "$BUILDKIT_IMAGE" --addr tcp://0.0.0.0:12341 >> "$ENV_OUT"
  # health: workers must answer over the TCP address before any probe
  # claims the lane; a container that never becomes healthy fails the run
  i=0
  until docker exec "$WORKER" buildctl --addr tcp://127.0.0.1:12341 debug workers >/dev/null 2>&1; do
    i=$((i + 1))
    if [ $i -ge 45 ]; then
      echo "FAIL: $WORKER never became healthy" >&2
      docker logs "$WORKER" | tail -30 >&2
      exit 1
    fi
    sleep 1
  done
  # A working daemon without an observed worker list and actual version
  # is not a qualified worker; plain-sh tee must not hide either failure.
  run_bridge "$RESULTS/worker-health.log" docker exec "$WORKER" \
    buildctl --addr tcp://127.0.0.1:12341 debug workers
  cat "$RESULTS/worker-health.log" >> "$ENV_OUT"
  run_bridge "$RESULTS/buildkit-version.log" docker exec "$WORKER" buildkitd --version
  cat "$RESULTS/buildkit-version.log" >> "$ENV_OUT"
}

worker_destroy() {
  docker rm -f -v "$WORKER" >/dev/null 2>&1 || true
  docker volume rm "$CACHE" >/dev/null 2>&1 || true
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
    if any(name in pkg["name"].lower()
           for name in ("buildkit", "moby", "docker", "containerd", "bollard"))
)
if builder:
    sys.exit(f"FAIL: native Rust lock gained builder dependencies: {builder}")
print("native Rust package graph: no builder dependencies")
PY
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
run_bridge "$RESULTS/probe2.log" "$BRIDGE" -addr "tcp://$PORT" -out "$RESULTS" -pins "$HERE/pins.env" -probe graph
run_bridge "$RESULTS/probe5.log" "$BRIDGE" -addr "tcp://$PORT" -out "$RESULTS" -pins "$HERE/pins.env" -probe policy
run_bridge "$RESULTS/probe3.log" "$BRIDGE" -addr "tcp://$PORT" -out "$RESULTS" -pins "$HERE/pins.env" -probe executable
WORKER1_SECONDS=$(( $(date +%s) - START ))

# --- probe 3 teardown: retained output survives worker+cache death --
worker_destroy
OUT=$("$HERE/results/executable/hello")
echo "probe3 retained run after worker+cache removal: $OUT" | tee -a "$RESULTS/probe3.log"
[ "$OUT" = "gripsack-b0-static-hello" ]

# --- probe 4: two FRESH workers, clean reproduction ------------------
worker_start
S1=$(date +%s)
run_bridge "$RESULTS/probe4-a.log" "$BRIDGE" -addr "tcp://$PORT" -out "$RESULTS/oci-a" -pins "$HERE/pins.env" -probe oci
A_SECONDS=$(( $(date +%s) - S1 ))
mv "$RESULTS/oci-a/oci-layout.tar" "$RESULTS/oci-1.tar"
worker_destroy

worker_start
S2=$(date +%s)
run_bridge "$RESULTS/probe4-b.log" "$BRIDGE" -addr "tcp://$PORT" -out "$RESULTS/oci-b" -pins "$HERE/pins.env" -probe oci
B_SECONDS=$(( $(date +%s) - S2 ))
mv "$RESULTS/oci-b/oci-layout.tar" "$RESULTS/oci-2.tar"
worker_destroy

echo "worker1-total-seconds: $WORKER1_SECONDS" >> "$ENV_OUT"
echo "oci-clean-build-a-seconds: $A_SECONDS" >> "$ENV_OUT"
echo "oci-clean-build-b-seconds: $B_SECONDS" >> "$ENV_OUT"

# Independent verifier failure must stop the harness before runtime
# promotion. A tee pipeline under plain sh would report tee's success.
run_bridge "$RESULTS/probe4-verify.json" python3 "$HERE/verify_oci.py" \
  "$RESULTS/oci-1.tar" "$RESULTS/oci-2.tar"

# Separate-runtime execution: neither of the torn-down BuildKit workers
# can substitute for a successful Docker-engine load and run.
if docker load -i "$RESULTS/oci-1.tar" > "$RESULTS/probe4-load.txt" 2>&1; then
  IMAGE_ID=$(sed -n 's/^Loaded image ID: sha256:\([0-9a-f]*\)$/\1/p' "$RESULTS/probe4-load.txt")
  if [ -n "$IMAGE_ID" ] && docker run --rm "$IMAGE_ID" cat /hello.txt > "$RESULTS/probe4-run.txt" 2>&1; then
    cat "$RESULTS/probe4-run.txt"
    docker rmi "$IMAGE_ID" >/dev/null
    echo "separate-runtime: docker engine (independent of both workers) loaded and ran the image" | tee "$RESULTS/probe4-runtime.txt"
  else
    echo "FAIL: separate-runtime image load succeeded but run failed (see probe4-load.txt)" | tee "$RESULTS/probe4-runtime.txt"
    exit 1
  fi
else
  echo "FAIL: separate-runtime docker load rejected the OCI layout (see probe4-load.txt)" | tee "$RESULTS/probe4-runtime.txt"
  exit 1
fi

echo "=== B0-01 Linux lane probes complete; results in $RESULTS"
echo "B0_LINUX_QUALIFICATION=6"
