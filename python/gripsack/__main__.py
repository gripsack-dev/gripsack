"""Eval entrypoint (plan/0005 §5, 0011 §5): `python -m gripsack <repo> --host <name>`.

Imports the env repo's modules and host entrypoint, runs registered
linters, then prints the eval envelope on stdout:

    {"ir": {...}, "diagnostics": [...]}

Any error-severity diagnostic fails eval (exit 1). The core never
embeds Python — it runs this as a subprocess and renders the
diagnostics itself.
"""

from __future__ import annotations

import argparse
import json
import sys
import types
from pathlib import Path

from .graph import emit_ir, registered_modules
from .lint import run_lints


def _exec(path: Path, name: str):
    # Never the bytecode cache: env-repo modules are rewritten by
    # scripts (grip apply loops, generators, git checkouts), and
    # CPython's pyc validation — mtime in whole seconds + file size —
    # treats a same-second, same-size rewrite as fresh. A stale module
    # then silently deploys stale config (observed in e2e: a template
    # vars change that never reached the core). Compile from source,
    # every time — modules are tiny.
    mod = types.ModuleType(name)
    mod.__file__ = str(path)
    exec(compile(path.read_bytes(), str(path), "exec"), mod.__dict__)
    return mod


def main() -> None:
    ap = argparse.ArgumentParser(prog="python -m gripsack")
    ap.add_argument("repo", help="env repo root (contains env.toml)")
    ap.add_argument("--host", default=None)
    ap.add_argument("--tags", default="", help="comma-separated extra tags")
    args = ap.parse_args()

    repo = Path(args.repo).resolve()
    tags = [t for t in args.tags.split(",") if t]

    # host entrypoint first: it declares tags (and later, module selection)
    if args.host:
        host_file = repo / "hosts" / f"{args.host}.py"
        if host_file.exists():
            host_mod = _exec(host_file, "gripsack_user.host")
            tags = list(getattr(host_mod, "tags", tags))

    if tags:
        from ._facts import _set_tags

        _set_tags(tags)

    modules_dir = repo / "modules"
    if modules_dir.is_dir():
        for f in sorted(modules_dir.glob("*.py")):
            _exec(f, f"gripsack_user.{f.stem}")

    diagnostics = run_lints(repo, args.host, registered_modules())
    payload = {
        "ir": json.loads(emit_ir(tags)),
        "diagnostics": [d.to_dict() for d in diagnostics],
    }
    sys.stdout.write(json.dumps(payload) + "\n")
    if any(d.severity == "error" for d in diagnostics):
        sys.exit(1)


if __name__ == "__main__":
    main()
