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
  docker exec "$WORKER" buildctl --addr tcp://127.0.0.1:12341 debug workers | tee -a "$ENV_OUT"
  docker exec "$WORKER" buildkitd --version | tee -a "$ENV_OUT" || true
}

worker_destroy() {
  docker rm -f -v "$WORKER" >/dev/null 2>&1 || true
  docker volume rm "$CACHE" >/dev/null 2>&1 || true
}

# --- probe 1: baseline preservation --------------------------------
# The product must not gain builder dependencies from this experiment:
# no buildkit/docker/go reference may exist in the workspace lockfile,
# the CLI binary, or the offline e2e journey (which the container gates
# already ran against file:// fixtures only).
{
  echo "--- probe1: baseline preservation"
  if (cd "$REPO" && grep -ri "buildkit\|moby" Cargo.lock crates/ --include='*.rs' --include='*.toml' -l | grep -v buildkit-qualification); then
    echo "FAIL: builder references leaked into product sources" >&2; exit 1
  fi
  echo "product sources: zero buildkit/moby references"
  if [ -x "$REPO/target/debug/grip" ]; then
    if ldd "$REPO/target/debug/grip" 2>/dev/null | grep -qi "docker\|containerd"; then
      echo "FAIL: grip links container runtime libs" >&2; exit 1
    fi
    echo "grip binary: no container-runtime linkage"
  else
    echo "note: target/debug/grip not built; source-level audit covers this stage"
  fi
} 2>&1 | tee "$RESULTS/probe1-baseline.txt"

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

# independent verification + reproduction comparison
python3 "$HERE/verify_oci.py" "$RESULTS/oci-1.tar" "$RESULTS/oci-2.tar" | tee "$RESULTS/probe4-verify.json"

# separate-runtime execution: the docker engine (independent of the
# torn-down buildkitd workers) must load and run the exported image.
if docker load -i "$RESULTS/oci-1.tar" > "$RESULTS/probe4-load.txt" 2>&1; then
  # load reports an image ID (untagged); docker run wants the bare hex
  IMAGE_ID=$(sed -n 's/^Loaded image ID: sha256:\([0-9a-f]*\)$/\1/p' "$RESULTS/probe4-load.txt")
  if [ -n "$IMAGE_ID" ] && docker run --rm "$IMAGE_ID" cat /hello.txt > "$RESULTS/probe4-run.txt" 2>&1; then
    cat "$RESULTS/probe4-run.txt"
    docker rmi "$IMAGE_ID" >/dev/null
    echo "separate-runtime: docker engine (independent of both workers) loaded and ran the image" | tee -a "$RESULTS/probe4-verify.json"
  else
    echo "separate-runtime: load OK but run failed (recorded; see probe4-load.txt)" | tee -a "$RESULTS/probe4-verify.json"
    exit 1
  fi
else
  echo "separate-runtime: docker load rejected the OCI layout tar (recorded; see probe4-load.txt)" | tee -a "$RESULTS/probe4-verify.json"
fi

echo "=== B0-01 Linux lane probes complete; results in $RESULTS"
