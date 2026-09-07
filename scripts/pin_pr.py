#!/usr/bin/env python3
"""Remote publication for pin-update proposals (scripts/check_pins.py --apply).

Everything happens on a throwaway `git worktree` of the default branch:
the local checkout — dirty, mid-flight, or on a feature branch — is
never touched or mutated. Idempotent by construction: the bot branch is
reset to the base before the edits are applied, PRs are looked up by
head branch, and gates are re-dispatched only when a new commit
actually landed. Never merges; a PR created with GITHUB_TOKEN does not
trigger CI by itself, so the proposer dispatches the two workflows that
already declare workflow_dispatch (ci.yml, repro.yml) on the candidate
branch — that needs only actions:write, no other convention."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import time
from datetime import datetime, timezone

REPO = "gripsack-dev/gripsack"
GATES = ("ci.yml", "repro.yml")
BOT = ("github-actions[bot]", "41898282+github-actions[bot]@users.noreply.github.com")


class PublishError(RuntimeError):
    pass


def _run(cmd: list[str], *, cwd: str | None = None, what: str) -> str:
    out = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd)
    if out.returncode != 0:
        raise PublishError(f"{what} failed: {(out.stderr or out.stdout).strip()[:400]}")
    return out.stdout.strip()


def git(args: list[str], cwd: str | None = None) -> str:
    return _run(["git", *args], cwd=cwd, what=f"git {args[0]}")


def gh(args: list[str], *, check: bool = True) -> str:
    out = subprocess.run(["gh", "-R", REPO, *args], capture_output=True, text=True)
    if check and out.returncode != 0:
        raise PublishError(f"gh {args[0]} failed: {(out.stderr or out.stdout).strip()[:400]}")
    return out.stdout.strip()


def apply_edits(edits, root: str) -> None:
    """Apply the unit's Edits inside `root`. Every edit's `old` must be
    found exactly `count` times in the running text (earlier edits in the
    same file apply first) — anything else fails closed instead of
    editing blind."""
    by_path: dict[str, list] = {}
    for e in edits:
        by_path.setdefault(e.path, []).append(e)
    for path, es in by_path.items():
        p = os.path.join(root, path)
        with open(p) as f:
            text = f.read()
        for e in es:
            n = text.count(e.old)
            if n != e.count:
                raise PublishError(
                    f"{path}: pin site mismatch — expected {e.count}× {e.old[:60]!r}, "
                    f"found {n}; refusing to edit blind (pin layout changed?)")
            text = text.replace(e.old, e.new)
        with open(p, "w") as f:
            f.write(text)


def _default_branch() -> str:
    return json.loads(gh(["repo", "view", "--json", "defaultBranchRef"]))["defaultBranchRef"]["name"]


def _run_link(workflow: str, branch: str, since: str) -> str | None:
    """Best-effort URL of the run dispatched just now (may not be indexed
    yet; absence only omits the link from the PR body)."""
    for _ in range(3):
        time.sleep(6)
        try:
            rows = json.loads(gh(["run", "list", "--workflow", workflow, "--branch", branch,
                                  "--limit", "5", "--json", "url,createdAt"], check=False) or "[]")
        except json.JSONDecodeError:
            rows = []
        for r in rows:
            if r.get("createdAt", "") >= since:
                return r["url"]
    return None


def _close_superseded(key: str, pr_url: str) -> None:
    rows = json.loads(gh(["issue", "list", "--search", f'in:title "pins: {key}"',
                          "--state", "open", "--json", "number"], check=False) or "[]")
    for r in rows:
        gh(["issue", "close", str(r["number"]),
            "--comment", f"Superseded by {pr_url} — the pin-update PR carries the real change."],
           check=False)


def publish(key: str, propose) -> str:
    """Push the unit's edits to pins/<key> (reset to the default branch),
    reuse-or-create the PR, dispatch the gates. `propose(root)` rebuilds
    the unit against the fetched base — the worktree parse is
    authoritative for the PR, not the local checkout. Returns the PR URL,
    or a no-op explanation. Raises PublishError on any failure."""
    branch = f"pins/{key}"
    base = _default_branch()
    repo_root = git(["rev-parse", "--show-toplevel"])
    git(["fetch", "origin", base], cwd=repo_root)

    tmp = tempfile.mkdtemp(prefix=f"pins-{key}-")
    try:
        git(["worktree", "add", "--detach", tmp, "FETCH_HEAD"], cwd=repo_root)
        unit = propose(tmp)  # re-parse pins from the base; may differ from the checkout
        if unit.status != "ready":
            return f"skipped — unit is {unit.status} on {base}: {unit.note}"
        apply_edits(unit.edits, tmp)
        git(["checkout", "-B", branch], cwd=tmp)
        if not git(["status", "--porcelain"], cwd=tmp):
            return "no changes — branch content already current (idempotent no-op)"
        git(["-c", f"user.name={BOT[0]}", "-c", f"user.email={BOT[1]}",
             "commit", "-am", f"pins: {unit.summary}\n\nPrepared by scripts/check_pins.py "
             "(upstream-watch, plan/0042 F) from official upstream metadata — deliberate, "
             "reviewed bump; never auto-merged."], cwd=tmp)
        git(["push", "--force", "origin", f"HEAD:{branch}"], cwd=tmp)

        since = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        dispatch_notes = []
        for wf in GATES:
            gh(["workflow", "run", wf, "--ref", branch])
            dispatch_notes.append((wf, _run_link(wf, branch, since)))
        links = "\n".join(f"- {wf}: {url}" if url else f"- {wf}: dispatched (run not indexed yet)"
                          for wf, url in dispatch_notes)

        existing = json.loads(gh(["pr", "list", "--head", branch, "--state", "open",
                                  "--json", "number,url"]) or "[]")
        if existing:
            url = existing[0]["url"]
            gh(["pr", "comment", str(existing[0]["number"]),
                "--body", f"Branch updated; gates re-dispatched:\n{links}"], check=False)
            return f"{url} (updated)"
        url = gh(["pr", "create", "--base", base, "--head", branch,
                  "--title", f"pins: {unit.summary}",
                  "--body", unit.pr_body() + f"\n\n**Gates dispatched on this branch**\n{links}"])
        _close_superseded(key, url)
        return url
    finally:
        subprocess.run(["git", "worktree", "remove", "--force", tmp],
                       capture_output=True, cwd=repo_root)
        shutil.rmtree(tmp, ignore_errors=True)
