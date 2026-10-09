#!/bin/sh
# Deterministic release artifacts for the BuildKit bridge helper, built
# from the pinned Go image (the pin lives in build.sh — single source).
# Every artifact is built TWICE and compared: a drift between the two
# means nondeterminism, which fails everything. Pins are MEASURED from
# the surviving artifacts — never fetched sidecars, never hand-entered.
#
#   dist.sh --dist DIR      build Linux x86_64 artifact + .sha256 into DIR
#   dist.sh --check         fail if the committed pins (bridge_pins.rs)
#                           drift from a fresh deterministic build
#   dist.sh --update-pins   run the full Go gate, then rewrite the
#                           committed pins (release step, after the
#                           integration owner declares the Go source
#                           stable — never speculative)
#
# VERSION defaults to the workspace crate version; the release workflow
# passes the tag's. The helper rides the core tag: assets land on
# core-v$VERSION next to the core tarballs.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO=$(CDPATH= cd -- "$HERE/../.." && pwd)
GOLANG_IMAGE=$(sed -n 's/^GOLANG_IMAGE=//p' "$HERE/build.sh")
PINS="$REPO/crates/gripsack-fetch/src/bridge_pins.rs"
VERSION=${VERSION:-$(sed -n 's/^version = "\([0-9.]*\)"$/\1/p' "$REPO/Cargo.toml" | head -1)}

# triple ↔ host.rs AssetTarget ↔ GOOS/GOARCH. The bridge is CGO-free,
# so the linux assets are libc-independent and serve the musl slots.
# REL-X64-FIRST-0460-2026-10-08; expand only with a qualified release scope.
TARGETS="
linux amd64 x86_64-unknown-linux-musl LinuxX86_64Musl
"

mode=--dist
dist=
while [ $# -gt 0 ]; do
  case $1 in
    --dist) shift; dist=$1 ;;
    --check|--update-pins) mode=$1 ;;
    *) echo "usage: dist.sh [--dist DIR | --check | --update-pins]" >&2; exit 2 ;;
  esac
  shift
done
[ -n "$dist" ] || dist=$(mktemp -d)
mkdir -p "$dist/a" "$dist/b"
# Docker treats a bare relative source as a named volume, not a host bind.
dist=$(CDPATH= cd -- "$dist" && pwd)

# Independent compiler caches keep the second round a fresh build, not reuse
# of the first round's already compiled artifact. Module download bytes are shared.
echo "$TARGETS" | while read -r goos goarch triple slot; do
  [ -n "$goos" ] || continue
  echo "grip-buildkit-bridge-$VERSION-$triple $goos $goarch"
done > "$dist/targets.txt"
docker run --rm \
  --user "$(id -u):$(id -g)" \
  -v "$REPO":/src -w /src/tools/buildkit-bridge \
  -v "$dist":/out \
  -e GOFLAGS=-mod=readonly -e GOTOOLCHAIN=local \
  -e GOPATH=/tmp/gopath \
  "$GOLANG_IMAGE" sh -ec '
    while read -r name goos goarch; do
      for round in a b; do
        CGO_ENABLED=0 GOOS=$goos GOARCH=$goarch GOCACHE="/tmp/gocache-$round" go build \
          -trimpath -buildvcs=false -ldflags="-buildid=" \
          -o "/out/$round/$name" .
      done
    done < /out/targets.txt
  '

# Determinism gate, then measure the surviving bytes.
while read -r name goos goarch; do
  cmp "$dist/a/$name" "$dist/b/$name" || {
    echo "::error::$name is not deterministic across identical builds" >&2; exit 1; }
  mv "$dist/a/$name" "$dist/$name"
  (cd "$dist" && sha256sum "$name" > "$name.sha256")
done < "$dist/targets.txt"
rm -rf "$dist/a" "$dist/b" "$dist/targets.txt"

hash_of() { # asset name → measured hash
  cut -d' ' -f1 < "$dist/$1.sha256"
}
pin_of() { # AssetTarget slot → exactly one committed pin, independent of rustfmt
  python3 - "$PINS" "$1" <<'PY'
from pathlib import Path
import re
import sys
pins = re.findall(r"\bAssetTarget::" + re.escape(sys.argv[2]) + r'\s*,\s*"([0-9a-f]{64})"', Path(sys.argv[1]).read_text())
if len(pins) != 1:
    raise SystemExit(f"expected exactly one measured pin for {sys.argv[2]}, found {len(pins)}")
print(pins[0])
PY
}

case $mode in
  --dist)
    echo "artifacts in $dist:"
    ls -l "$dist" | grep -v '^d' | grep grip-buildkit-bridge
    ;;
  --check)
    committed=$(sed -n 's/^pub(crate) const BRIDGE_VERSION: &str = "\(.*\)";$/\1/p' "$PINS")
    [ "$committed" = "$VERSION" ] || {
      echo "::error::bridge_pins.rs version $committed != $VERSION" >&2; exit 1; }
    while read -r goos goarch triple slot; do
      [ -n "$goos" ] || continue
      name="grip-buildkit-bridge-$VERSION-$triple"
      measured=$(hash_of "$name")
      pinned=$(pin_of "$slot")
      if [ "$measured" != "$pinned" ]; then
        echo "::error::pin drift for $slot ($triple): committed ${pinned:-<none>} != measured $measured" >&2
        exit 1
      fi
    done <<EOF
$TARGETS
EOF
    echo "bridge pins verified: committed == deterministic measured build for $VERSION"
    ;;
  --update-pins)
    # Pin only gated source: format + vet + race + build must pass first.
    "$HERE/build.sh"
    {
      echo "//! GENERATED — measured per-platform pins for the BuildKit bridge helper."
      echo "//!"
      echo "//! \`tools/buildkit-bridge/dist.sh --update-pins\` rewrites this file from"
      echo "//! a deterministic build inside the pinned golang image: every hash is"
      echo "//! the measured sha256 of the exact artifact that ships on the"
      echo "//! \`core-v<BRIDGE_VERSION>\` GitHub release — never a fetched sidecar"
      echo "//! checksum. The release gate rebuilds and compares every platform."
      echo
      echo "use crate::host::AssetTarget;"
      echo
      echo "/// The core release whose \`core-v<version>\` tag carries the matching"
      echo "/// helper artifacts (parent decision: helper rides the core tag, no"
      echo "/// separate namespace). Flipping this without re-measuring the hashes"
      echo "/// below fails the release workflow's \`--check\`."
      echo "pub(crate) const BRIDGE_VERSION: &str = \"$VERSION\";"
      echo
      echo "/// (platform, sha256 of \`grip-buildkit-bridge-<version>-<triple>\`)."
      echo "/// Generated only from the measured, gated helper source."
      echo "pub(crate) const BRIDGE_SHA256: &[(AssetTarget, &str)] = &["
      while read -r goos goarch triple slot; do
        [ -n "$goos" ] || continue
        name="grip-buildkit-bridge-$VERSION-$triple"
        echo "    (AssetTarget::$slot, \"$(hash_of "$name")\"),"
      done <<EOF
$TARGETS
EOF
      echo "];"
    } > "$PINS.new"
    mv "$PINS.new" "$PINS"
    echo "pins updated in $PINS for $VERSION — review, commit, then tag"
    ;;
esac
