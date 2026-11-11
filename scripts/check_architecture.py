#!/usr/bin/env python3
"""Enforce the workspace dependency direction (plan/0056 §6).

Dependencies flow strictly inward: `gripsack → exec/fetch → host →
kernel`, and `ir → kernel`. Every workspace member declares the
workspace crates it may depend on (`INWARD`); an edge outside that set,
an undeclared member, or a retired crate (`RETIRED`, dissolved in F2
step 10b) reappearing as a member or a dependency is a violation. The
kernel, the host and the IR additionally have closed third-party
allowlists (`EXTERNAL`), and no workspace member may pull an
openssl/native-tls stack (rustls-only, AGENTS.md). The checked universe
comes from the root `[workspace]` membership globs — never a `crates/`
directory listing — so members living outside `crates/` (the fuzz
crate) cannot escape any rule. Manifest parsing (including renamed and
target-specific dependencies) avoids a source-text grep.
"""
from __future__ import annotations

import argparse
import copy
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

# The workspace crates each member may depend on. Ring 0 (kernel) has none;
# ring 2 (host) only the kernel; the IR only the kernel; ring-3 crates
# (fetch, conda, exec, config, trace, griplint) the host, the IR and the
# sources below them; the CLI (ring 4) anything. New edges require a
# deliberate reviewed update of this map.
INWARD: dict[str, frozenset[str]] = {
    "gripsack-kernel": frozenset(),
    "gripsack-host": frozenset({"gripsack-kernel"}),
    "gripsack-ir": frozenset({"gripsack-kernel"}),
    "gripsack-fetch": frozenset({"gripsack-host", "gripsack-ir", "gripsack-kernel"}),
    "gripsack-conda": frozenset({"gripsack-fetch", "gripsack-host", "gripsack-ir", "gripsack-kernel"}),
    "gripsack-config": frozenset({"gripsack-ir"}),
    "gripsack-trace": frozenset({"gripsack-host"}),
    "griplint": frozenset({"gripsack-ir"}),
    "gripsack-exec": frozenset({
        "gripsack-conda", "gripsack-config", "gripsack-fetch", "gripsack-host", "gripsack-ir",
        "gripsack-kernel",
    }),
    "gripsack": frozenset({
        "griplint", "gripsack-conda", "gripsack-config", "gripsack-exec", "gripsack-fetch",
        "gripsack-host", "gripsack-ir", "gripsack-kernel", "gripsack-trace",
    }),
    # The fuzz harness drives the host's codecs, the fetchers and the IR.
    "gripsack-fuzz": frozenset({"gripsack-fetch", "gripsack-host", "gripsack-ir"}),
}
# Dissolved into gripsack-host, -fetch, -ir, -exec and -conda (plan/0056 §24).
RETIRED = frozenset({"gripsack-fs", "gripsack-policy", "gripsack-process", "gripsack-store"})
# Closed third-party allowlists for the inner rings.
EXTERNAL: dict[str, frozenset[str]] = {
    # plan/0056: the ring-0 kernel has no dependencies at all.
    "gripsack-kernel": frozenset(),
    # Ring 2: cap-std/rustix/libc for the fs and the process supervisor;
    # serde/sha2/getrandom for the store's wire codecs and identities.
    "gripsack-host": frozenset({
        "cap-std", "rustix", "libc", "serde", "serde_json", "sha2", "tempfile", "getrandom",
    }),
    "gripsack-ir": frozenset({
        "serde", "serde_json", "thiserror", "jsonschema",
        # Pure semantic identities, workspace admission and credential-free URI parsing.
        "sha2", "url",
    }),
}
WORKSPACE = frozenset(INWARD) | RETIRED
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
    issues: list[str] = []
    if crate in RETIRED:
        issues.append(f"{crate}: retired crate is a workspace member again")
    elif crate not in INWARD:
        issues.append(f"{crate}: workspace member has no declared ring in INWARD")
    inward = INWARD.get(crate, frozenset())
    external = EXTERNAL.get(crate)
    for section, package in dependencies(manifest):
        if package in FORBIDDEN_TLS:
            issues.append(f"{crate} {section}: {package} violates rustls-only policy")
        elif package in RETIRED:
            issues.append(f"{crate} {section}: {package} is a retired crate")
        elif package in WORKSPACE:
            if package not in inward:
                issues.append(f"{crate} {section}: {package} breaks the inward dependency direction")
        elif external is not None and package not in external:
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
    for protected in EXTERNAL:
        if protected not in members:
            issues.append(f"required protected crate missing from workspace members: {protected}")
    return issues


def resolved_tls(manifest: Path, target: str | None) -> list[str]:
    """Check Cargo's active normal/build graph, including optional-helper deps."""
    command = ["cargo", "tree", "--locked", "--manifest-path", str(manifest),
               "--edges", "normal,build", "--prefix", "none", "--format", "{p}"]
    if target:
        command.extend(["--target", target])
    listing = subprocess.check_output(command, text=True)
    packages = {line.split()[0] for line in listing.splitlines() if line.strip()}
    return [f"{manifest}: active {package} violates rustls-only policy"
            for package in sorted(packages & FORBIDDEN_TLS)]


def write_crate(directory: Path, name: str, extra: str = "") -> None:
    """One scratch crate manifest; `extra` is verbatim TOML appended
    after the package header (dependency sections under test)."""
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "Cargo.toml").write_text(
        f'[package]\nname = "{name}"\nversion = "0.0.0"\n{extra}'
    )


def scratch_workspace(
    root: Path,
    *,
    crates: tuple[str, ...] = tuple(EXTERNAL),
    extra: dict[str, str] | None = None,
    fuzz_extra: str = "",
) -> None:
    """A minimal mirror of the real workspace shape: crates under
    `crates/` plus the fuzz member outside it, so calibration exercises
    member-glob discovery rather than a directory listing."""
    extra = extra or {}
    root.mkdir(parents=True, exist_ok=True)
    (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*", "fuzz"]\n')
    for name in crates:
        write_crate(root / "crates" / name, name, extra.get(name, ""))
    write_crate(root / "fuzz", "gripsack-fuzz", fuzz_extra)


def self_check() -> list[str]:
    """Negative calibration: every mutated workspace must fail for the
    named property; the clean control must pass, or the harness itself
    is blind."""
    failures: list[str] = []

    def expect(label: str, issues: list[str], needle: str) -> None:
        if not any(needle in issue for issue in issues):
            failures.append(f"calibration failed: {label}")

    # Rule-level calibration against synthetic manifests.
    host = {"package": {"name": "gripsack-host"}, "dependencies": {"serde": {"workspace": True}}}
    outward = copy.deepcopy(host)
    outward["dependencies"]["gripsack-exec"] = {"path": "../gripsack-exec"}
    expect(
        "outward host -> exec edge admitted",
        violations(outward, "gripsack-host"),
        "breaks the inward dependency direction",
    )
    renamed = copy.deepcopy(host)
    renamed["target"] = {"cfg(unix)": {"dependencies": {"fetch_alias": {"package": "gripsack-fetch", "path": "../gripsack-fetch"}}}}
    expect(
        "renamed target-specific host -> fetch edge admitted",
        violations(renamed, "gripsack-host"),
        "breaks the inward dependency direction",
    )
    host_ir = copy.deepcopy(host)
    host_ir["dependencies"]["gripsack-ir"] = {"path": "../gripsack-ir"}
    expect(
        "host -> ir edge admitted (ring 2 depends only on the kernel)",
        violations(host_ir, "gripsack-host"),
        "breaks the inward dependency direction",
    )
    ir_host = {"package": {"name": "gripsack-ir"}, "dependencies": {"gripsack-host": {"path": "../gripsack-host"}}}
    expect(
        "ir -> host edge admitted (the IR depends only on the kernel)",
        violations(ir_host, "gripsack-ir"),
        "breaks the inward dependency direction",
    )
    fetch_exec = {"package": {"name": "gripsack-fetch"}, "dependencies": {"gripsack-exec": {"path": "../gripsack-exec"}}}
    expect(
        "fetch -> exec edge admitted",
        violations(fetch_exec, "gripsack-fetch"),
        "breaks the inward dependency direction",
    )
    exec_cli = {"package": {"name": "gripsack-exec"}, "dependencies": {"gripsack": {"path": "../gripsack"}}}
    expect(
        "exec -> CLI edge admitted",
        violations(exec_cli, "gripsack-exec"),
        "breaks the inward dependency direction",
    )
    retired = {"package": {"name": "gripsack-exec"}, "dependencies": {"gripsack-store": {"path": "../gripsack-store"}}}
    expect(
        "retired crate admitted as a dependency",
        violations(retired, "gripsack-exec"),
        "retired crate",
    )
    host_network = copy.deepcopy(host)
    host_network["dependencies"]["ureq"] = {"version": "2"}
    expect(
        "network client admitted in the host",
        violations(host_network, "gripsack-host"),
        "crosses the protected domain boundary",
    )
    kernel = {"package": {"name": "gripsack-kernel"}, "dependencies": {"serde": {"workspace": True}}}
    expect(
        "third-party dependency admitted in the kernel",
        violations(kernel, "gripsack-kernel"),
        "crosses the protected domain boundary",
    )
    aliased_tls = copy.deepcopy(host)
    aliased_tls["dependencies"]["tls"] = {"package": "openssl", "version": "0.10"}
    expect(
        "openssl admitted under a renamed dependency alias",
        violations(aliased_tls, "gripsack-host"),
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

        host_build_edge = base / "host_build_edge"
        scratch_workspace(
            host_build_edge,
            extra={"gripsack-host": "[target.'cfg(unix)'.build-dependencies]\ngripsack-exec = { path = \"../gripsack-exec\" }\n"},
        )
        expect(
            "executor edge admitted through a target-specific build-dependency",
            check(host_build_edge),
            "breaks the inward dependency direction",
        )

        revived = base / "revived"
        scratch_workspace(revived, crates=(*EXTERNAL, "gripsack-policy"))
        expect(
            "retired crate admitted as a workspace member",
            check(revived),
            "retired crate is a workspace member again",
        )

        undeclared = base / "undeclared"
        scratch_workspace(undeclared, crates=(*EXTERNAL, "gripsack-sources"))
        expect(
            "workspace member without a declared ring admitted",
            check(undeclared),
            "no declared ring",
        )

        missing_host = base / "missing_host"
        scratch_workspace(missing_host, crates=("gripsack-kernel", "gripsack-ir"))
        expect(
            "workspace without the host crate admitted",
            check(missing_host),
            "required protected crate missing",
        )

    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--self-check", action="store_true", help="calibrate forbidden dependency rejection")
    parser.add_argument("--resolved-manifest", action="append", type=Path, default=[],
                        help="also inspect this Cargo manifest's active normal/build graph")
    parser.add_argument("--target", help="target triple for resolved dependency checks")
    args = parser.parse_args()
    issues = check(args.root)
    for manifest in args.resolved_manifest:
        issues.extend(resolved_tls(manifest, args.target))
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
        print("ok: direction, boundary, retired, undeclared, rustls-only, member-discovery and missing-crate negatives all rejected")
    print("ok: dependencies flow inward (gripsack -> exec/fetch -> host -> kernel, ir -> kernel)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
