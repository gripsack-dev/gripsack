#!/bin/sh
# Canonical verification gate entrypoint (plans 0046 and 0048 §6).
# Pins move together: Verus 0.2026.09.06.8dea4a2, Rust 1.98.0,
# Z3 4.16.0 and vstd =0.0.0-2026-09-06-0133 (Dockerfile/Cargo.toml).
# Structured solver results, named family floors, and diagnostic attribution
# live in the Python driver; unrelated failures never count as calibration.
set -eu
exec python3 "$(dirname "$0")/check_verus.py"
