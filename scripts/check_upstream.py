#!/usr/bin/env python3
"""Weekly freshness watch: for each data pack, compare the
tool's latest upstream release against the pack's supported set.
Tracks unsupported releases on one open refresh issue per pack."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from urllib.parse import quote
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


def issue_rows(endpoint: str) -> list[dict]:
    """Read every page before treating an empty result as permission to create."""
    pages = subprocess.check_output(
        ["gh", "api", "--paginate", f"repos/{REPO}/issues{endpoint}", "--jq", "@json"],
        text=True,
    ).splitlines()
    rows = []
    for line in pages:
        page = json.loads(line)
        if not isinstance(page, list):
            raise ValueError(f"unexpected issue listing for {endpoint}")
        rows.extend(page)
    if not pages:
        raise ValueError(f"missing issue listing for {endpoint}")
    return rows


def report(issues: list[dict], package: str, repo: str, tag: str, supported: list[str]) -> None:
    title = f"{package}: tables lag upstream"
    key = f"<!-- griplint-pack: {package} -->"
    observation = f"{key}\nLatest unsupported release: <https://github.com/{repo}/releases/tag/{quote(tag, safe='')}>"
    matches = []
    for issue in issues:
        body = issue["body"] or ""
        # Adopt existing tag-titled reports, including retitled reports whose
        # original generated introduction survives. Never rewrite their text.
        if (key in body.splitlines() or issue["title"] == title or issue["title"].startswith(title + " ")
                or (f"Upstream {repo} released **" in body and f"but the `{package}` data pack supports" in body)):
            matches.append(issue)
    if len(matches) > 1:
        numbers = ", ".join(f"#{issue['number']}" for issue in matches)
        raise ValueError(f"{package}: multiple open refresh issues ({numbers}); refusing to choose")
    if not matches:
        body = (f"{observation}\n\nThe `{package}` data pack supports `{supported}`. "
                f"Refresh `crates/griplint/packs/{package}.toml` and its fixtures per "
                "`.agents/skills/griplint-author/SKILL.md`.")
        subprocess.run(["gh", "issue", "create", "-R", REPO, "--title", title, "--body", body,
                        "--label", "linter", "--label", "help wanted", "--label", "needs-triage"], check=True)
        print(f"  + {package}: opened issue for {tag}")
        return

    issue, = matches
    number = issue["number"]
    comments = issue_rows(f"/{number}/comments?per_page=100")
    last = next((c["body"] for c in reversed(comments) if c["body"].startswith(key + "\n")), None)
    if last is None:
        body = issue["body"] or ""
        already_seen = (observation in body or issue["title"] == f"{title} {tag}"
                        or f"Upstream {repo} released **{tag}**, but the `{package}` data pack supports" in body)
    else:
        already_seen = last == observation
    if already_seen:
        print(f"  = {package}: #{number} already tracks {tag}")
    else:
        subprocess.run(["gh", "issue", "comment", str(number), "-R", REPO, "--body", observation], check=True)
        print(f"  ~ {package}: recorded {tag} on #{number}")


def main() -> None:
    issues = None
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
        if issues is None:
            issues = [row for row in issue_rows("?state=open&per_page=100") if "pull_request" not in row]
        report(issues, package, repo, tag, supported)


if __name__ == "__main__":
    try:
        main()
    except (subprocess.CalledProcessError, ValueError) as error:
        sys.exit(f"upstream watch: {error}")
