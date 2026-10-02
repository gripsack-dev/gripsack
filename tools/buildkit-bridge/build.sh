#!/bin/sh
# Pinned Go/BuildKit gate. No host toolchain contract, fuzz or corpus replay.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO=$(CDPATH= cd -- "$HERE/../.." && pwd)
GOLANG_IMAGE=golang:1.26.3@sha256:e3665e241a474aba30bbfaf177cfa88e1913e970c83bd86889cacfb67d6e7e51
docker run --rm \
  --user "$(id -u):$(id -g)" \
  -v "$REPO":/src -w /src/tools/buildkit-bridge \
  -e GOFLAGS=-mod=readonly -e GOTOOLCHAIN=local \
  -e GOPATH=/tmp/gopath -e GOCACHE=/tmp/gocache \
  "$GOLANG_IMAGE" sh -ec '
    unformatted=$(gofmt -l .)
    if [ -n "$unformatted" ]; then
      printf "gofmt required:\n%s\n" "$unformatted" >&2
      exit 1
    fi
    go vet ./...
    go test -race ./...
    CGO_ENABLED=0 go build -trimpath -buildvcs=false -o bridge-bin .
  '
echo "bridge gate passed: format + vet + race tests + production binary"
