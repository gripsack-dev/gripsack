#!/bin/sh
# Rebuild the B0 bridge inside the pinned golang container — the host
# needs no Go toolchain at build or run time. Module hashes are pinned
# by go.sum; the image by digest in ../pins.env.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC1091
. "$HERE/../pins.env"
docker run --rm \
  --user "$(id -u):$(id -g)" \
  -v "$HERE":/src:ro -v "$HERE":/out -w /src \
  -e GOFLAGS=-mod=readonly -e GOTOOLCHAIN=local -e GOCACHE=/tmp/gocache \
  -e GOPATH=/tmp/gopath -e HOME=/tmp/home -e CGO_ENABLED=0 \
  "$GOLANG_IMAGE" sh -ec 'test -z "$(gofmt -l *.go)" && go mod tidy -diff && go build -trimpath -buildvcs=false -o /out/bridge-bin .'
echo "built $HERE/bridge-bin"
