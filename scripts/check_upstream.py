#!/usr/bin/env python3
"""Weekly freshness watch: for each data pack, compare the
tool's latest upstream release against the pack's supported set.
Opens an issue per laggard (idempotent by title)."""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import urllib.request

REPO = "gripsack-dev/gripsack"

TOOLS = {
    "helix": "helix-editor/helix",
    "yazi": "sxyazi/yazi",
    "starship": "starship/starship",
    "zola": "getzola/zola",
    "git-cliff": "orhun/git-cliff",
    "bottom": "ClementTsang/bottom",
    "atuin": "atuinsh/atuin",
    "jj": "jj-vcs/jj",
    "procs": "dalance/procs",
    "bacon": "Canop/bacon",
    "rio": "raphamorim/rio",
    "harlequin": "tconbeer/harlequin",
    "television": "alexpasmantier/television",
    "broot": "Canop/broot",
    "mise": "jdx/mise",
    "alacritty": "alacritty/alacritty",
    "superfile": "yorukot/superfile",
    "glow": "charmbracelet/glow",
    "ruff": "astral-sh/ruff",
    "zed": "zed-industries/zed",
    "gh-dash": "dlvhdr/gh-dash",
    "tuicr": "agavra/tuicr",
    "claude-code": "anthropics/claude-code",
}


def latest_release(repo: str) -> str | None:
    req = urllib.request.Request(
        f"https://api.github.com/repos/{repo}/releases/latest",
        headers={"Accept": "application/vnd.github+json", "User-Agent": "griplint-upstream-watch"},
    )
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return json.load(r)["tag_name"]
    except Exception as e:
        print(f"  ! {repo}: {e}")
        return None


def supported_prefixes(tool: str) -> list[str]:
    """The pack's supported version prefixes (crates/griplint/packs)."""
    import tomllib

    with open(f"crates/griplint/packs/{tool}.toml", "rb") as f:
        return tomllib.load(f)["meta"]["supported"]


def main() -> None:
    for package, repo in sorted(TOOLS.items()):
        tag = latest_release(repo)
        if not tag:
            continue
        supported = supported_prefixes(package)
        version = tag.lstrip("v")
        covered = any(version.startswith(p) for p in supported)
        if covered:
            print(f"  ✓ {package}: {tag} covered")
            continue
        title = f"{package}: tables lag upstream {tag}"
        existing = subprocess.run(
            ["gh", "issue", "list", "-R", REPO, "--search", f"in:title {title}", "--state", "open", "--json", "number"],
            capture_output=True, text=True,
        ).stdout
        if json.loads(existing or "[]"):
            print(f"  = {package}: issue already open for {tag}")
            continue
        subprocess.run(
            ["gh", "issue", "create", "-R", REPO, "--title", title,
             "--label", "linter", "--label", "help wanted",
             "--body", f"Upstream {repo} released **{tag}**, but the `{package}` data pack supports `{supported}`.\n\nRefresh the pack per `.agents/skills/griplint-author/SKILL.md` (research the release's config changes, update `crates/griplint/packs/{package}.toml`, bump `supported`, update fixtures in `crates/griplint/fixtures/{package}/`)."],
            check=True,
        )
        print(f"  + {package}: opened issue for {tag}")


if __name__ == "__main__":
    sys.exit(main())
