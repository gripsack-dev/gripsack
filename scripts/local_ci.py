#!/usr/bin/env python3
"""Local qualification gates: thin wrappers over docker compose services.

`.ci/pilot.json` lists the gates `python3 -m ci_infra qualify` runs, each in a
fresh committed checkout; the same commands work in any developer checkout.
Nothing here publishes.

    test      fmt + clippy -D warnings + cargo test (compose `test`) + script unit tests
    ts-test   deno frontend tests and example type-checks (compose `ts-test`)
    e2e       flow tests against the real debug binary (compose `e2e`)
    audit     cargo audit of the core and helper lockfiles (compose `audit`)
    package   release.py prepare: core tarball, Conda helper (pin-checked), SDK, installer
    smoke     release.py verify: static binary, embedded SBOM, versions, checksums and the
              installed-consumer smoke against the pinned website/example repositories
    proofs    compose `proofs` (plan/0056 §7); not yet a required gate
"""
from __future__ import annotations

import argparse
from pathlib import Path
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def compose(service):
    return [["docker", "compose", "run", "--build", "--rm", service]]


GATES = {
    "test": compose("test") + [[sys.executable, "-m", "unittest", "discover", "-s", "scripts", "-p", "test_*.py"]],
    "ts-test": compose("ts-test"),
    "e2e": compose("e2e"),
    "audit": compose("audit"),
    "package": [[sys.executable, "scripts/release.py", "prepare"]],
    "smoke": [[sys.executable, "scripts/release.py", "verify"]],
    "proofs": compose("proofs"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("gate", choices=GATES)
    for command in GATES[parser.parse_args().gate]:
        print("+ " + shlex.join(command), flush=True)
        if subprocess.run(command, cwd=ROOT).returncode:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
