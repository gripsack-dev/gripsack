"""Typed fetchers (0001 §3.1, 0002 §2)."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Any, Optional


class FetchKind(str, Enum):
    """The closed set of fetch kinds the engine interprets."""

    GITHUB_RELEASE = "github_release"
    TARBALL = "tarball"
    GIT = "git"
    FILE = "file"
    PLUGIN = "plugin"
    BREW = "brew"
    PIXI = "pixi"


@dataclass(frozen=True)
class Fetch:
    """A fetch spec — the engine interprets it; never a script."""

    kind: FetchKind
    args: dict[str, Any]

    def __post_init__(self) -> None:
        object.__setattr__(self, "kind", FetchKind(self.kind))

    def to_ir(self) -> dict[str, Any]:
        return {"kind": self.kind.value, **self.args}


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
    return Fetch(FetchKind.GITHUB_RELEASE, args)


def tarball(url: str, sha256: Optional[str] = None) -> Fetch:
    args: dict[str, Any] = {"url": url}
    if sha256 is not None:
        args["sha256"] = sha256
    return Fetch(FetchKind.TARBALL, args)


def git(url: str, rev: str) -> Fetch:
    return Fetch(FetchKind.GIT, {"url": url, "rev": rev})


def file_fetch(path: str) -> Fetch:
    return Fetch(FetchKind.FILE, {"path": path})


def plugin_fetch(name: str, **args: Any) -> Fetch:
    """A fetcher plugin transport (0002 §4) — `gripfetch-<name>`."""
    return Fetch(FetchKind.PLUGIN, {"name": name, "args": args})


def brew(formula: str, version: Optional[str] = None) -> Fetch:
    """A Homebrew bottle — resolved from the formula JSON, so the pin
    (bottle sha256) needs no download at update time.

    `version` is a tripwire, not a range: brew only ever serves the
    *current* formula, so a mismatch fails at resolve with
    "`grip update` to move" rather than a sha mismatch later."""
    args: dict[str, Any] = {"formula": formula}
    if version:
        args["version"] = version
    return Fetch(FetchKind.BREW, args)


def pixi(package: str, version: Optional[str] = None) -> Fetch:
    """A conda package via pixi, installed into an isolated PIXI_HOME
    and harvested into the store."""
    args: dict[str, Any] = {"package": package}
    if version is not None:
        args["version"] = version
    return Fetch(FetchKind.PIXI, args)
