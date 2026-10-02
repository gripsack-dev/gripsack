#!/bin/sh
# Native musl-static Linux x64/ARM64 and native macOS ARM64 (Intel optional).
# Invoke: sh tools/conda-helper/dist.sh --dist /absolute/output --target TRIPLE [--check]
# Without --check this only measures; --check fails closed on missing/drifted pins.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 "$HERE/package.py" "$@"
