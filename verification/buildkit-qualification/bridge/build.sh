#!/bin/sh
# Rebuild the B0 bridge inside the pinned golang container — the host
# needs no Go toolchain at build or run time. Module hashes are pinned
# by go.sum; the image by digest in ../pins.env.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC1091
. "$HERE/../pins.env"
docker run --rm --network=host \
  -v "$HERE":/src -w /src \
  -e GOFLAGS=-mod=mod -e GOTOOLCHAIN=local -e GOCACHE=/tmp/gocache \
  "$GOLANG_IMAGE" sh -c 'gofmt -w *.go && go mod tidy && go build -o bridge-bin .'
echo "built $HERE/bridge-bin"
