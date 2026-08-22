"""Typed fetchers (0001 §3.1, 0002 §2)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional


@dataclass(frozen=True)
class Fetch:
    """A fetch spec — the engine interprets it; never a script."""

    kind: str
    args: dict[str, Any]

    def to_ir(self) -> dict[str, Any]:
        return {"kind": self.kind, **self.args}


def github_release(
    repo: str,
    asset: str,
    version: Optional[str] = None,
    sha256: Optional[str] = None,
    base_url: Optional[str] = None,
) -> Fetch:
    """GitHub releases; `base_url` covers GitHub Enterprise (0002 rung 1)."""
    args: dict[str, Any] = {"repo": repo, "asset": asset}
    if version is not None:
        args["version"] = version
    if sha256 is not None:
        args["sha256"] = sha256
    if base_url is not None:
        args["base_url"] = base_url
    return Fetch("github_release", args)


def tarball(url: str, sha256: Optional[str] = None) -> Fetch:
    args: dict[str, Any] = {"url": url}
    if sha256 is not None:
        args["sha256"] = sha256
    return Fetch("tarball", args)


def git(url: str, rev: str) -> Fetch:
    return Fetch("git", {"url": url, "rev": rev})


def file_fetch(path: str) -> Fetch:
    return Fetch("file", {"path": path})


def plugin_fetch(name: str, **args: Any) -> Fetch:
    """A fetcher plugin transport (0002 §4) — `gripfetch-<name>`."""
    return Fetch("plugin", {"name": name, "args": args})
