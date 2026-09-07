#!/usr/bin/env bash
# 0042 E fuzz gate entrypoint. Deterministic replay by default; the
# longer coverage-guided mode is opt-in for scheduled workflows.
#
#   FUZZ_TOOLCHAIN=<exact rustup pin> scripts/check_fuzz.sh [replay|scheduled]
#
# FUZZ_SECONDS (scheduled mode only, default 60, max 3600) bounds each
# target's libFuzzer run; fuzz/run.py enforces it. Containment is
# bubblewrap — there is no uncontained fallback, on CI or anywhere.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
mode="${1:-replay}"
case "$mode" in
    replay|scheduled) ;;
    *) echo "usage: $0 [replay|scheduled]" >&2; exit 64 ;;
esac

if [ -z "${FUZZ_TOOLCHAIN:-}" ]; then
    echo "FUZZ_TOOLCHAIN must name the exact pinned rustup toolchain" >&2
    exit 64
fi
for tool in bwrap cargo python3; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "$tool is required by the fuzz gate" >&2
        exit 69
    }
done

# The scheduled build compiles libfuzzer-sys's bundled C++ runtime; the
# replay build does not need it.
if [ "$mode" = "scheduled" ] && ! command -v c++ >/dev/null 2>&1; then
    echo "scheduled mode needs a C++ compiler for libFuzzer (c++ not found)" >&2
    exit 69
fi

cd "$root"
exec python3 fuzz/run.py "$mode"
