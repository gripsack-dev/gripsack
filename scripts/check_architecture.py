#!/usr/bin/env python3
"""Enforce the A1 domain-crate dependency direction (plan/0052 §3).

Policy kernels, IR admission and the durable store cannot depend on
execution/fetcher/backend/OS-manager crates, and no workspace member
may pull an openssl/native-tls stack (rustls-only, AGENTS.md). The
checked universe comes from the root `[workspace]` membership globs —
never a `crates/` directory listing — so members living outside
`crates/` (the fuzz crate) cannot escape either rule. Manifest parsing
(including renamed and target-specific dependencies) avoids a
source-text grep; the existing Rust schema_acceptance corpus separately
checks the IR schema against the production parser in the required
test gate.
"""
from __future__ import annotations

import argparse
import copy
import sys
import tempfile
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


def member_manifests(root: Path) -> list[Path]:
    """Manifest of every cargo workspace member, from the root
    `[workspace]` membership globs (with `exclude` applied).

    A bare `crates/` listing would silently skip members declared
    elsewhere — `members = ["crates/*", "fuzz"]` today — leaving them
    outside the rustls-only and protected-boundary rules.
    """
    root_manifest_path = root / "Cargo.toml"
    with root_manifest_path.open("rb") as manifest_file:
        root_manifest = tomllib.load(manifest_file)
    workspace = root_manifest.get("workspace", {})
    excluded = {
        entry for pattern in workspace.get("exclude", []) for entry in root.glob(pattern)
    }
    manifests: list[Path] = []
    for pattern in workspace.get("members", []):
        for member in sorted(root.glob(pattern)):
            if member in excluded:
                continue
            manifest_path = member / "Cargo.toml"
            if manifest_path.is_file():
                manifests.append(manifest_path)
    if "package" in root_manifest:
        manifests.append(root_manifest_path)
    return sorted(set(manifests))


def check(root: Path) -> list[str]:
    issues: list[str] = []
    members: set[str] = set()
    for manifest_path in member_manifests(root):
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        crate = manifest["package"]["name"]
        members.add(crate)
        issues.extend(violations(manifest, crate))
    for protected in ALLOWED:
        if protected not in members:
            issues.append(f"required protected crate missing from workspace members: {protected}")
    return issues


def write_crate(crates_dir: Path, name: str, extra: str = "") -> None:
    """One scratch crate manifest; `extra` is verbatim TOML appended
    after the package header (dependency sections under test)."""
    crate_dir = crates_dir / name
    crate_dir.mkdir(parents=True, exist_ok=True)
    (crate_dir / "Cargo.toml").write_text(
        f'[package]\nname = "{name}"\nversion = "0.0.0"\n{extra}'
    )


def scratch_workspace(
    root: Path,
    *,
    protected: tuple[str, ...] = tuple(ALLOWED),
    store_extra: str = "",
    fuzz_extra: str = "",
) -> None:
    """A minimal mirror of the real workspace shape: the protected
    crates under `crates/` plus the fuzz member outside it, so
    calibration exercises member-glob discovery rather than a
    directory listing."""
    root.mkdir(parents=True, exist_ok=True)
    (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*", "fuzz"]\n')
    for name in protected:
        write_crate(root / "crates", name, store_extra if name == "gripsack-store" else "")
    write_crate(root, "fuzz", fuzz_extra)


def self_check() -> list[str]:
    """Negative calibration: every mutated workspace must fail for the
    named property; the clean control must pass, or the harness itself
    is blind."""
    failures: list[str] = []

    def expect(label: str, issues: list[str], needle: str) -> None:
        if not any(needle in issue for issue in issues):
            failures.append(f"calibration failed: {label}")

    # Rule-level calibration against synthetic manifests.
    manifest = {"package": {"name": "gripsack-store"}, "dependencies": {"serde": {"workspace": True}}}
    forbidden = copy.deepcopy(manifest)
    forbidden["dependencies"]["gripsack-fetch"] = {"path": "../gripsack-fetch"}
    expect(
        "fetcher dependency admitted in durable store",
        violations(forbidden, "gripsack-store"),
        "crosses the protected domain boundary",
    )
    renamed = copy.deepcopy(manifest)
    renamed["target"] = {"cfg(unix)": {"dependencies": {"solver_alias": {"package": "gripsack-fetch", "path": "../gripsack-fetch"}}}}
    expect(
        "renamed target-specific fetcher dependency admitted",
        violations(renamed, "gripsack-store"),
        "crosses the protected domain boundary",
    )
    aliased_tls = copy.deepcopy(manifest)
    aliased_tls["dependencies"]["tls"] = {"package": "openssl", "version": "0.10"}
    expect(
        "openssl admitted under a renamed dependency alias",
        violations(aliased_tls, "gripsack-store"),
        "rustls-only",
    )

    # End-to-end calibration through check(): real TOML on scratch
    # trees exercises member-glob discovery, tomllib parsing and the
    # protected-crate presence rule, not just violations().
    with tempfile.TemporaryDirectory() as scratch:
        base = Path(scratch)

        clean = base / "clean"
        scratch_workspace(clean)
        control_issues = check(clean)
        if control_issues:
            failures.append(f"calibration failed: clean workspace control rejected: {control_issues}")

        fuzz_tls = base / "fuzz_tls"
        scratch_workspace(fuzz_tls, fuzz_extra='[dependencies]\nopenssl = "0.10"\n')
        expect(
            "openssl admitted in the fuzz workspace member (outside crates/)",
            check(fuzz_tls),
            "rustls-only",
        )

        store_build_edge = base / "store_build_edge"
        scratch_workspace(
            store_build_edge,
            store_extra="[target.'cfg(unix)'.build-dependencies]\ngripsack-exec = { path = \"../gripsack-exec\" }\n",
        )
        expect(
            "executor edge admitted through a target-specific build-dependency",
            check(store_build_edge),
            "crosses the protected domain boundary",
        )

        missing_store = base / "missing_store"
        scratch_workspace(missing_store, protected=("gripsack-policy", "gripsack-ir"))
        expect(
            "workspace without the durable-store crate admitted",
            check(missing_store),
            "required protected crate missing",
        )

    return failures


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
        failures = self_check()
        if failures:
            for failure in failures:
                print(failure, file=sys.stderr)
            return 1
        print("ok: boundary, rustls-only, member-discovery and missing-crate negatives all rejected")
    print("ok: policy, IR and store dependency boundaries are closed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
