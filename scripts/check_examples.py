#!/usr/bin/env python3
"""Executable published TypeScript examples (plan/0042 §E).

The important TypeScript examples on the website are extracted from
the site source at run time and evaluated for real — no manually
maintained shadow copy of example code anywhere in this harness:

  - the PACKAGED SDK (--sdk, an npm tarball of @gripsack/core) is
    installed into every fixture repo's node_modules/@gripsack/core —
    the deliberate-pin rule — so the package's export surface is what
    serves every example, at both levels;
  - frontend level: the SDK driver, sourced from the core repo
    (typescript/src/cli.ts — exactly what the core spawns), runs each
    scaffolded repo under the core's sandbox flags;
  - core level: the real `grip check` (deliberately pinned SDK and sandboxed
    Deno) must accept the same root workspace.

Both levels use offline fixtures and an isolated HOME. Illustrative
fragments are classified explicitly with evidence — never silently
treated as passing executable examples, and never an excuse to drop a
complete example that merely needs fixtures (the harness provides
them). Examples are paired to the manifest by location only; the CODE
always comes from the docs, so a new or removed doc example fails
pairing until the manifest is deliberately re-classified.

Usage (core CI / local — candidate SDK package + core):

    cd typescript && npm ci && npm run build && npm pack   # candidate
    python3 scripts/check_examples.py \
        --site <gripsack-dev.github.io checkout> --core-repo . \
        --sdk typescript/gripsack-core-<version>.tgz \
        [--grip target/debug/grip] [--deno $(which deno)] [--selfcheck]

For published artifacts, use the matching SDK archive attached to the release.
These checks run locally; the website's Pages job only deploys static files.

Input contract:
  --site DIR       website checkout (default: sibling gripsack-site
                   directory of the core repo, or $GRIPSACK_SITE)
  --core-repo DIR  core repo (default: this script's repo root);
                   --grip defaults to $GRIPSACK_BIN, then
                   <core-repo>/target/debug/grip, then release
  --sdk TGZ       REQUIRED: the packed @gripsack/core npm tarball the
                   examples must consume; its version must equal the
                   driver tree's package.json version
  --frontend DIR   driver sources (deno.json + src/cli.ts); default
                   <core-repo>/typescript
  --deno BIN       deno binary (default: $GRIPSACK_DENO, then PATH)
  --grip BIN       grip binary under test
  --pixi-fixture DIR  override the checked-in real Pixi manifest/lock pair
                     used for the explicit-import example
  --selfcheck      prove a missing root-workspace return is rejected by
                   evaluation behavior, not source-text matching
  --only ID        run a single manifest entry (repeatable)
  --list           print extraction + classification, run nothing
  --keep           keep scratch dirs on success too
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from examples_check import evaluate, extract, fixtures, selfchecks
from examples_check.extract import CheckFailure
from examples_check.manifest import EXAMPLES

REPO_ROOT = Path(__file__).resolve().parent.parent


def find_deno(explicit: str | None) -> Path:
    cand = explicit or os.environ.get("GRIPSACK_DENO") or shutil.which("deno")
    if not cand:
        raise CheckFailure(
            "no deno found — pass --deno, set GRIPSACK_DENO, or put deno "
            "on PATH (CI pins 2.9.6, the DENO_RELEASE version)"
        )
    return Path(cand).resolve()


def find_grip(explicit: str | None, core_repo: Path) -> Path:
    cands = [explicit, os.environ.get("GRIPSACK_BIN"),
             core_repo / "target/debug/grip", core_repo / "target/release/grip"]
    for c in cands:
        if c and Path(c).exists():
            return Path(c).resolve()
    raise CheckFailure(
        "no grip binary — pass --grip, set GRIPSACK_BIN, or build one "
        f"({core_repo}/target/debug/grip)"
    )


def find_site(explicit: str | None, core_repo: Path) -> Path:
    cand = explicit or os.environ.get("GRIPSACK_SITE") or (core_repo.parent / "gripsack-site")
    if (Path(cand) / "doc").is_dir():
        return Path(cand)
    raise CheckFailure(
        f"{cand} is not a website checkout (no doc/) — pass --site"
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--site")
    ap.add_argument("--core-repo", default=str(REPO_ROOT))
    ap.add_argument("--sdk",
                    help="packed @gripsack/core npm tarball (npm pack output); "
                         "required unless --list")
    ap.add_argument("--frontend")
    ap.add_argument("--pixi-fixture", type=Path,
                    default=Path(__file__).resolve().parent / "examples_check" / "pixi",
                    help="override the checked-in real pixi.toml and pixi.lock pair")
    ap.add_argument("--grip")
    ap.add_argument("--deno")
    ap.add_argument("--selfcheck", action="store_true",
                    help="also prove the missing-return regression is caught")
    ap.add_argument("--only", action="append", dest="only")
    ap.add_argument("--list", action="store_true",
                    help="print extraction + classification, run nothing")
    ap.add_argument("--keep", action="store_true",
                    help="keep scratch dirs on success too")
    args = ap.parse_args()

    core_repo = Path(args.core_repo).resolve()
    site = find_site(args.site, core_repo)
    failed = False
    try:
        blocks = extract.pair_with_manifest(extract.extract_blocks(site, core_repo), EXAMPLES)
    except CheckFailure as e:
        print(f"FAIL {e}", file=sys.stderr)
        return 1

    selected = [e for e in EXAMPLES if not args.only or e["id"] in args.only]
    missing = set(args.only or []) - {e["id"] for e in selected}
    if missing:
        print(f"FAIL unknown --only ids: {sorted(missing)}", file=sys.stderr)
        return 1

    if args.list:
        for entry in selected:
            b = blocks[entry["id"]]
            loc = b.title or f"fence@{b.line}"
            print(f"{entry['id']:<38} {entry['kind']:<8} {b.file}:{b.line}  ({loc})")
            if entry["kind"] == "fragment":
                print(f"    fragment: {entry['reason']}")
        runnable = sum(1 for e in selected if e["kind"] != "fragment")
        print(f"\n{len(selected)} examples: {runnable} runnable, "
              f"{len(selected) - runnable} classified fragments")
        return 0

    if args.sdk is None:
        print("FAIL --sdk is required: the packed @gripsack/core npm tarball "
              "(cd typescript && npm ci && npm run build && npm pack)", file=sys.stderr)
        return 1
    if any(e.get("pixi_fixture") for e in selected):
        try:
            pair = {name: (args.pixi_fixture / name).read_text(encoding="utf-8")
                    for name in ("pixi.toml", "pixi.lock")}
        except OSError as e:
            print(f"FAIL reading Pixi fixture: {e}", file=sys.stderr)
            return 1
        selected = [dict(e, fixtures=pair) if e.get("pixi_fixture") else e
                    for e in selected]
    sdk_src = Path(args.frontend).resolve() if args.frontend else core_repo / "typescript"
    if not (sdk_src / "deno.json").is_file() or not (sdk_src / "src" / "cli.ts").is_file():
        print(f"FAIL {sdk_src} has no driver (need deno.json + src/cli.ts)", file=sys.stderr)
        return 1
    frontend_version = json.loads(
        (sdk_src / "package.json").read_text(encoding="utf-8")
    )["version"]

    base = Path(tempfile.mkdtemp(prefix="grip-examples-"))
    try:
        try:
            ctx = {
                "facts": evaluate.host_facts(),
                "sdk_src": sdk_src,
                "deno": find_deno(args.deno),
                "grip": find_grip(args.grip, core_repo),
                "base": base,
            }
            ctx["sdk_pkg"] = fixtures.prepare_sdk(
                Path(args.sdk).resolve(), base, frontend_version)
        except CheckFailure as e:
            print(f"FAIL {e}", file=sys.stderr)
            return 1


        failures = selfchecks.pin_canary(ctx)
        canary_ok = not failures
        if not canary_ok:
            failed = True

        pkg_version = fixtures.sdk_version_of(ctx["sdk_pkg"])
        print(f"checker: {len(selected)} examples · sdk package "
              f"@gripsack/core@{pkg_version} ({args.sdk}) · driver {sdk_src} · "
              f"grip {ctx['grip']} · deno {ctx['deno']}")
        if canary_ok:
            print("  ok pin canary: the installed node_modules package answers "
                  "(ir_version 999 marker)")
        else:
            for f in failures:
                print(f"  ! {f}")
        for entry in selected:
            block = blocks[entry["id"]]
            if entry["kind"] == "fragment":
                print(f"  - {entry['id']}: fragment (classified, not executed)")
                print(f"      {entry['reason']}")
                continue
            failures = evaluate.run_example(entry, block, ctx)
            if args.selfcheck:
                failures += selfchecks.selfcheck_workspace(entry, block, ctx)
            if failures:
                failed = True
                print(f"  ! {entry['id']}: FAIL")
                for f in failures:
                    print(f"      {f}")
            else:
                extra = " (+selfcheck)" if args.selfcheck else ""
                print(f"  ok {entry['id']}: eval + grip check accept{extra}")
    finally:
        if failed or args.keep:
            print(f"\nscratch kept at {base}")
        else:
            shutil.rmtree(base, ignore_errors=True)

    runnable = sum(1 for e in selected if e["kind"] != "fragment")
    fragments = len(selected) - runnable
    verdict = "FAILED" if failed else "ok"
    print(f"\n{verdict}: {runnable} executable examples evaluated "
          f"(packaged SDK via deliberate pin + real grip check), "
          f"{fragments} fragments classified")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
