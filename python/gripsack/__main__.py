"""Eval entrypoint (plan/0005 §4): `python -m gripsack <repo> --host <name>`.

Imports the env repo's modules and host entrypoint, then prints the IR
on stdout. The core never embeds Python — it runs this as a subprocess.
"""

from __future__ import annotations

import argparse
import importlib.util
import sys
from pathlib import Path

from .graph import emit_ir


def _exec(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
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

    sys.stdout.write(emit_ir(tags) + "\n")


if __name__ == "__main__":
    main()
