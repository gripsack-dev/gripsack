#!/usr/bin/env python3
"""Enforce the A1 domain-crate dependency direction (plan/0052 §3).

Policy kernels, IR admission and the durable store cannot depend on
execution/fetcher/backend/OS-manager crates. Manifest parsing (including
renamed and target-specific dependencies) avoids a source-text grep;
the existing Rust schema_acceptance corpus separately checks the IR
schema against the production parser in the required test gate.
"""
from __future__ import annotations

import argparse
import copy
import sys
import tomllib
from pathlib import Path

# Each protected crate owns one domain/effect boundary. New direct
# dependencies require a deliberate reviewed update of this map.
ALLOWED: dict[str, frozenset[str]] = {
    "gripsack-policy": frozenset({"vstd"}),
    "gripsack-ir": frozenset({"gripsack-policy", "serde", "serde_json", "thiserror", "jsonschema"}),
    "gripsack-store": frozenset({
        "gripsack-fs", "gripsack-ir", "gripsack-policy", "serde", "serde_json",
        "toml", "sha2", "tempfile",
    }),
}
FORBIDDEN_TLS = frozenset({"openssl", "openssl-sys", "native-tls"})
SECTIONS = ("dependencies", "build-dependencies", "dev-dependencies")


def dependencies(manifest: dict) -> list[tuple[str, str]]:
    """(section, actual Cargo package) across ordinary and target deps."""
    seen: list[tuple[str, str]] = []

    def collect(group: dict, prefix: str) -> None:
        for section in SECTIONS:
            for alias, spec in group.get(section, {}).items():
                package = spec.get("package", alias) if isinstance(spec, dict) else alias
                seen.append((f"{prefix}{section}", package))

    collect(manifest, "")
    for target, sections in manifest.get("target", {}).items():
        collect(sections, f"target.{target}.")
    return seen


def violations(manifest: dict, crate: str) -> list[str]:
    accepted = ALLOWED.get(crate)
    issues: list[str] = []
    for section, package in dependencies(manifest):
        if package in FORBIDDEN_TLS:
            issues.append(f"{crate} {section}: {package} violates rustls-only policy")
        elif accepted is not None and package not in accepted:
            issues.append(f"{crate} {section}: {package} crosses the protected domain boundary")
    return issues


def check(root: Path) -> list[str]:
    issues: list[str] = []
    crates = root / "crates"
    for crate_dir in sorted(crates.iterdir()):
        manifest_path = crate_dir / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        crate = manifest["package"]["name"]
        issues.extend(violations(manifest, crate))
    for protected in ALLOWED:
        if not (crates / protected / "Cargo.toml").is_file():
            issues.append(f"required protected crate missing: {protected}")
    return issues


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--self-check", action="store_true", help="calibrate forbidden dependency rejection")
    args = parser.parse_args()
    issues = check(args.root)
    if issues:
        for issue in issues:
            print(f"architecture violation: {issue}", file=sys.stderr)
        return 1
    if args.self_check:
        manifest = {"package": {"name": "gripsack-store"}, "dependencies": {"serde": {"workspace": True}}}
        forbidden = copy.deepcopy(manifest)
        forbidden["dependencies"]["gripsack-fetch"] = {"path": "../gripsack-fetch"}
        if not any("crosses the protected domain boundary" in issue for issue in violations(forbidden, "gripsack-store")):
            print("calibration failed: fetcher dependency admitted in durable store", file=sys.stderr)
            return 1
        renamed = copy.deepcopy(manifest)
        renamed["target"] = {"cfg(unix)": {"dependencies": {"solver_alias": {"package": "gripsack-fetch", "path": "../gripsack-fetch"}}}}
        if not any("crosses the protected domain boundary" in issue for issue in violations(renamed, "gripsack-store")):
            print("calibration failed: renamed target-specific fetcher dependency admitted", file=sys.stderr)
            return 1
        print("ok: forbidden direct and renamed target-specific edges rejected")
    print("ok: policy, IR and store dependency boundaries are closed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
