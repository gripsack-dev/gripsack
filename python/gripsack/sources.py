"""Typed fetchers (0001 §3.1, 0002 §2)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional


@dataclass(frozen=True)
class Source:
    """A typed fetcher — the engine interprets it; never a script."""

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
) -> Source:
    """GitHub releases; `base_url` covers GitHub Enterprise (0002 rung 1)."""
    args: dict[str, Any] = {"repo": repo, "asset": asset}
    if version is not None:
        args["version"] = version
    if sha256 is not None:
        args["sha256"] = sha256
    if base_url is not None:
        args["base_url"] = base_url
    return Source("github_release", args)


def tarball(url: str, sha256: Optional[str] = None) -> Source:
    args: dict[str, Any] = {"url": url}
    if sha256 is not None:
        args["sha256"] = sha256
    return Source("tarball", args)


def git(url: str, rev: str) -> Source:
    return Source("git", {"url": url, "rev": rev})


def file_source(path: str) -> Source:
    return Source("file", {"path": path})


def plugin_source(name: str, **args: Any) -> Source:
    """A sourcerer plugin transport (0002 §4)."""
    return Source("plugin", {"name": name, "args": args})
