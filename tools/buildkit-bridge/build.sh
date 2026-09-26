#!/bin/sh
# Build and test the production bridge inside the pinned golang
# container — the host needs no Go toolchain (same contract as the B0
# qualification harness). The shared protocol conformance corpus is
# mounted from the repo root so both implementations decode identical
# wire bytes. Stdlib only for now: go.sum pins arrive with the first
# upstream BuildKit dependency (client/lower/client, B1-03 transport).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
GOLANG_IMAGE=golang:1.26.3@sha256:e3665e241a474aba30bbfaf177cfa88e1913e970c83bd86889cacfb67d6e7e51
docker run --rm --network=host \
  -v "$REPO":/src -w /src/tools/buildkit-bridge \
  -e GOFLAGS=-mod=mod -e GOTOOLCHAIN=local -e GOCACHE=/tmp/gocache \
  "$GOLANG_IMAGE" sh -c \
  'gofmt -l . | grep . && { echo "gofmt needed"; exit 1; } || true;
   go vet ./... && go test ./...'
echo "bridge protocol gate passed"
