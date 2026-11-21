#!/usr/bin/env python3
"""Pin freshness watch + deliberate pin-update PR proposals (RFC 0042 F).

Every pinned toolchain, base-image digest and tool version is compared
against its OFFICIAL upstream metadata. Where a bump has complete
authoritative data (digests, asset checksums, version metadata) the
script prepares REAL source pin changes and — in --apply mode — lands
them as a reviewed PR on a stable `pins/<name>` bot branch via gh.
Never a silent floating upgrade, never auto-merged, never a
TODO-only proposal file: the PR contains the actual pin edits.

Modes:
  (default)  read-only report: per-pin status + the exact diffs a PR
             would contain. No git/gh writes at all.
  --apply    authorized CI only (gh + GH_TOKEN/GITHUB_TOKEN): edits are
             prepared on a throwaway `git worktree` of the default branch
             (the local checkout is never touched, dirty or not), pushed
             to pins/<name>, and the PR opened/reused by head branch.
             Exact-source local qualification is required before integration;
             no hosted build or fuzz/corpus replay is dispatched.
             Pins that cannot move yet fail closed as manual-work issues.
             Idempotent.

Coupled pins move as ONE proposal each:
  rust    Dockerfile rust:alpine digest, GNU_RUST_IMAGE, FUZZ_TOOLCHAIN
          and the Conda helper's matching builder/compiler pins. The
          alpine digest must match the version-specific alpine manifest.
          The proof-extraction compiler has its own independent pin.
  deno    Dockerfile (image tag/digest, DENO_VERSION, DENO_SHA256) and
          host.rs DENO_RELEASE (version and Linux x86_64 asset checksum).
          There is no hosted Deno toolchain pin.
  tools   cargo-auditable (version + both musl tarball sha256s), tla2tools
          (version + jar sha256), uv (version + Linux x86_64 wheel hash),
          node (pinned major + Linux x64 distribution checksum).
  images  other digest-pinned FROM lines (eclipse-temurin, python) —
          tag-scoped digests coupled to nothing else.

Metadata sources (official, read-only): Docker Hub registry API
(manifest digests), static.rust-lang.org stable channel, GitHub
releases API (latest tags + asset sha256 digests), deno release
.zip.sha256sum sidecars, PyPI JSON (uv wheels), nodejs.org dist index and SHASUMS256.txt.

Deliberately unsupported (fail closed, the issue asks for manual work):
a checksum/digest upstream does not publish at any of those sources
(e.g. a missing deno sidecar or GitHub asset digest), an image not yet
republished for the new toolchain, pin sites diverging among themselves
with no upstream bump to ride on (never auto-downgrade), or a pin site
that moved from its expected spelling (repo layout changed). A pin
uniformly ahead of upstream latest is deliberate and left alone.
Checksums are never computed ad hoc and pins are never partially
decoupled to get a PR through."""

from __future__ import annotations

import argparse
import difflib
import json
import os
import re
import subprocess
import sys
import tomllib
import urllib.parse
import urllib.request
from dataclasses import dataclass, field

REPO = "gripsack-dev/gripsack"

UA = {"User-Agent": "gripsack-pin-watch"}
MANIFEST_ACCEPT = ", ".join([
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
])

HELPER_PACKAGE = "tools/conda-helper/package.py"
VERSION = r"[0-9]+\.[0-9]+\.[0-9]+"
SHA256 = r"[0-9a-f]{64}"
HOST_RS = "crates/gripsack-fetch/src/host.rs"

# deno asset name per host.rs AssetTarget slot (glibc builds for the
# linux slots — deno ships no musl build, host.rs maps them)
DENO_TARGETS = {
    "LinuxX86_64Musl": "x86_64-unknown-linux-gnu",
}


# --- official metadata (read-only) ---------------------------------------

def fetch_json(url: str, headers: dict[str, str] | None = None) -> object:
    req = urllib.request.Request(url, headers={"Accept": "application/json", **UA, **(headers or {})})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def github_api(path: str) -> object:
    headers = {"Accept": "application/vnd.github+json"}
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return fetch_json(f"https://api.github.com{path}", headers)


def github_latest(repo: str) -> str | None:
    """Latest stable release tag (the /latest endpoint excludes prereleases)."""
    try:
        return github_api(f"/repos/{repo}/releases/latest")["tag_name"]
    except Exception as e:
        print(f"  ! {repo}: {e}")
        return None


def github_asset_digests(repo: str, tag: str) -> dict[str, str] | None:
    """Asset name -> sha256 from the release's own metadata (the API's
    digest field is what upstream published, not something we computed)."""
    try:
        rel = github_api(f"/repos/{repo}/releases/tags/{tag}")
        out = {a["name"]: (a.get("digest") or "").removeprefix("sha256:")
               for a in rel.get("assets", [])}
        return out if out else None
    except Exception as e:
        print(f"  ! {repo}@{tag}: {e}")
        return None


def docker_hub_digest(repository: str, tag: str) -> str | None:
    """The manifest digest currently published for a tag (Docker Hub)."""
    try:
        scope = urllib.parse.quote(f"repository:{repository}:pull")
        tok = fetch_json(f"https://auth.docker.io/token?service=registry.docker.io&scope={scope}")["token"]
        req = urllib.request.Request(
            f"https://registry-1.docker.io/v2/{repository}/manifests/{tag}",
            headers={"Accept": MANIFEST_ACCEPT, "Authorization": f"Bearer {tok}", **UA},
        )
        with urllib.request.urlopen(req, timeout=30) as r:
            digest = re.fullmatch(rf"sha256:({SHA256})", r.headers.get("Docker-Content-Digest", ""))
            return digest[1] if digest else None
    except Exception as e:
        print(f"  ! {repository}:{tag}: {e}")
        return None


def channel_stable() -> tuple[str, str] | None:
    """(short, full) for current stable rustc, e.g. ("1.98.1", "1.98.1 (48a… 2026-09-01)")."""
    try:
        with urllib.request.urlopen("https://static.rust-lang.org/dist/channel-rust-stable.toml", timeout=30) as r:
            full = tomllib.load(r)["pkg"]["rust"]["version"]
        match = re.fullmatch(rf"({VERSION}) \([0-9a-f]+ \d{{4}}-\d{{2}}-\d{{2}}\)", full)
        return (match[1], full) if match else None
    except Exception as e:
        print(f"  ! rust stable channel: {e}")
        return None


def deno_sidecar_sha(version: str, triple: str) -> str | None:
    """Per-asset sha256 from deno's release sidecars — the same source
    host.rs's pin comment names. No download-and-hash fallback: if the
    sidecar is missing, the checksum work is manual, never fabricated."""
    url = (f"https://github.com/denoland/deno/releases/download/"
           f"v{version}/deno-{triple}.zip.sha256sum")
    try:
        text = urllib.request.urlopen(
            urllib.request.Request(url, headers=UA), timeout=30).read().decode()
        m = re.search(r"\b[0-9a-f]{64}\b", text)
        return m.group(0) if m else None
    except Exception:
        return None


def uv_wheel(version: str) -> str | None:
    """Digest of the unique broadest-manylinux x86_64 wheel, from PyPI."""
    try:
        candidates = []
        legacy = {"manylinux1_x86_64": 5, "manylinux2010_x86_64": 12, "manylinux2014_x86_64": 17}
        for wheel in fetch_json(f"https://pypi.org/pypi/uv/{version}/json")["urls"]:
            name = wheel["filename"]
            prefix = f"uv-{version}-py3-none-"
            if not name.startswith(prefix) or not name.endswith(".whl") or wheel.get("yanked"):
                continue
            floors = []
            for tag in name[len(prefix):-4].split("."):
                if m := re.fullmatch(r"manylinux_2_([0-9]+)_x86_64", tag):
                    floors.append(int(m[1]))
                elif tag in legacy:
                    floors.append(legacy[tag])
            if floors:
                candidates.append((min(floors), wheel["digests"]["sha256"]))
        if candidates:
            lowest = min(floor for floor, _ in candidates)
            winners = [sha for floor, sha in candidates if floor == lowest]
            if len(winners) == 1 and re.fullmatch(SHA256, winners[0]):
                return winners[0]
    except Exception as e:
        print(f"  ! uv {version}: {e}")
    return None


def _line(text: str, prefix: str) -> str:
    """Return the unique active source line with this prefix, or fail closed."""
    matches = [line for line in text.splitlines() if line.startswith(prefix)]
    return matches[0] if len(matches) == 1 else ""


def vtuple(version: str) -> tuple[int, ...]:
    return tuple(int(x) for x in re.findall(r"\d+", version)[:4])


# --- current pins, read from the files that own them ---------------------

class PinSites:
    """Parses pins from their owning files under `root` (the checkout for
    the report; the fetched base worktree when preparing a PR)."""

    def __init__(self, root: str = "."):
        self.root = root
        self._files: dict[str, str] = {}

    def text(self, path: str) -> str:
        if path not in self._files:
            with open(os.path.join(self.root, path)) as source:
                self._files[path] = source.read()
        return self._files[path]

    def dockerfile_arg(self, name: str) -> str | None:
        m = re.search(rf"^ARG {re.escape(name)}=(\S+)", self.text("Dockerfile"), re.M)
        return m.group(1) if m else None

    def dockerfile_froms(self) -> dict[str, tuple[str, str]]:
        """name -> (tag, digest) for every digest-pinned FROM line."""
        out = {}
        for ref, dig in re.findall(r"^FROM (\S+)@sha256:([0-9a-f]{64})", self.text("Dockerfile"), re.M):
            name, tag = ref.rsplit(":", 1)
            out[name] = (tag, dig)
        return out

    def host_deno(self) -> tuple[str, dict[str, str]] | None:
        """(version, {AssetTarget: sha256}) from host.rs DENO_RELEASE."""
        src = self.text(HOST_RS)
        m = re.search(r"pub const DENO_RELEASE: ToolRelease = ToolRelease \{(.*?)\n\};", src, re.S)
        if not m:
            return None
        ver = re.search(r'version:\s*"([^"]+)"', m.group(1))
        pairs = re.findall(r"AssetTarget::(\w+),\s*\n?\s*\"([0-9a-f]{64})\"", m.group(1))
        if not ver or len(pairs) != len(DENO_TARGETS) or set(dict(pairs)) != set(DENO_TARGETS):
            return None
        return ver.group(1), dict(pairs)


# --- proposals ------------------------------------------------------------

@dataclass
class Edit:
    """One exact-text replacement; `count` is the number of occurrences
    that MUST be found — anything else fails closed instead of editing
    blind."""
    path: str
    old: str
    new: str
    count: int = 1


@dataclass
class PinUnit:
    key: str                 # stable bot-branch suffix (pins/<key>)
    summary: str = ""        # PR title suffix / drift headline
    status: str = "current"  # current | ready | blocked | unknown
    note: str = ""
    issue_title: str = ""    # set when blocked+drift needs a manual issue
    manual: str = ""         # that issue's body
    edits: list[Edit] = field(default_factory=list)
    sources: list[str] = field(default_factory=list)  # metadata provenance for the PR body

    def blocked(self, note: str, title: str) -> "PinUnit":
        """Fail closed: drift exists but the bump cannot be prepared —
        the issue this opens asks for the manual work instead."""
        self.status, self.note, self.issue_title = "blocked", note, title
        self.manual = note + "\n\n" + MANUAL_WORK
        return self

    def sites_missing(self) -> "PinUnit":
        return self.blocked(f"{self.key} pin sites not found where expected — parser needs a look",
                            f"pins: {self.key}: pin sites not found where expected")

    def propose_lines(self, sites: PinSites, *changes: tuple[str, str, str]) -> PinUnit:
        """Only propose the unit if each changed line can be replaced unambiguously."""
        edits = []
        for path, old, new in changes:
            if old == new:
                continue
            text = sites.text(path)
            left = "" if text.startswith(old + "\n") or text == old else "\n"
            right = "" if text.endswith("\n" + old) or text == old else "\n"
            source = left + old + right
            if text.count(source) != 1:
                return self.sites_missing()
            edits.append(Edit(path, source, left + new + right))
        if edits:
            self.status, self.edits = "ready", edits
        return self

    def pr_body(self) -> str:
        lines = [
            f"### pins: {self.summary}",
            "",
            "Deliberate pin update (RFC 0042 F), prepared by `scripts/check_pins.py`",
            "(upstream-watch) from official upstream metadata only. Never a silent",
            "floating upgrade, never auto-merged — review and merge deliberately.",
            "",
        ]
        if self.sources:
            lines.extend(f"- {source}" for source in self.sources)
            lines.append("")
        lines += [
            "**Review checklist**",
            "- [ ] the sources above still resolve to these exact values",
            "- [ ] coupled pin sites all moved together in this PR",
            "- [ ] exact-source local qualification, including the required",
            "      reproducibility gate, passes on this candidate",
            "",
            "No hosted build is dispatched; this proposal is not qualification evidence.",
        ]
        return "\n".join(lines)


MANUAL_WORK = (
    "This pin cannot move automatically yet — the pin-update automation "
    "(scripts/check_pins.py, RFC 0042 F) fails closed here rather than "
    "fabricate a checksum or decouple pins that must move together. "
    "Update deliberately per [RFC 0042 §F](https://github.com/gripsack-dev/gripsack/blob/main/docs/rfcs/2026-09-07-bounded-acquisition-and-complete-pins.md): verify from official metadata "
    "(digest/manifest check, `rustc -vV`, asset sha256), move every "
    "coupled pin site in one reviewed PR, run "
    "`scripts/check_reproducible.sh` before and after.")


def unit_rust(sites: PinSites) -> PinUnit:
    u = PinUnit(key="rust", summary="rust toolchain")
    df = sites.text("Dockerfile")
    try:
        helper = sites.text(HELPER_PACKAGE)
    except FileNotFoundError:
        return u.sites_missing()
    musl = _line(df, "FROM rust:")
    gnu = _line(df, "ARG GNU_RUST_IMAGE=")
    fuzz = _line(df, "ENV FUZZ_TOOLCHAIN=")
    image = _line(helper, "LINUX_IMAGE = ")
    compiler = _line(helper, "COMPILER = ")
    m = re.fullmatch(rf"FROM (rust:alpine@sha256:({SHA256})) AS builder", musl)
    g = re.fullmatch(rf"ARG GNU_RUST_IMAGE=rust:({VERSION})@sha256:({SHA256})", gnu)
    if not (m and g and _line(df, "FROM ${GNU_RUST_IMAGE} ") == "FROM ${GNU_RUST_IMAGE} AS local-rust-tools"):
        return u.sites_missing()
    pinned = g[1]
    if (fuzz != f"ENV FUZZ_TOOLCHAIN={pinned}" or compiler != f'COMPILER = "{pinned}"'
            or image != f'LINUX_IMAGE = "{m[1]}"'):
        return u.blocked("Builder, fuzz and helper pins disagree; refusing a partial compiler update",
                         "pins: rust: coupled pins missing or diverged")

    stable = channel_stable()
    if not stable or not re.fullmatch(VERSION, stable[0]):
        u.status, u.note = "unknown", "stable channel metadata unavailable"
        return u
    new = max((pinned, stable[0]), key=vtuple)  # never downgrade a deliberate pin
    registry = "https://registry-1.docker.io/v2/library/rust/manifests"
    digests = {}
    for tag in ("alpine", f"{new}-alpine", new):
        digest = docker_hub_digest("library/rust", tag)
        if not re.fullmatch(SHA256, digest or ""):
            return u.blocked(f"No valid manifest digest for rust:{tag}; no partial compiler update",
                             f"pins: rust: image metadata for {new} unavailable")
        digests[tag] = digest
        u.sources.append(f"{registry}/{tag} — `sha256:{digest}`")
    if (digests["alpine"] != digests[f"{new}-alpine"] or
            (new != pinned and (digests["alpine"] == m[2] or digests[new] == g[2]))):
        return u.blocked(f"The musl/GNU image digests do not establish the move to rustc {new}",
                         f"pins: rust: builder images not ready for {new}")
    u.sources.append(f"https://static.rust-lang.org/dist/channel-rust-stable.toml — stable `{stable[1]}`")
    new_image = f"rust:alpine@sha256:{digests['alpine']}"
    u.propose_lines(sites,
        ("Dockerfile", musl, f"FROM {new_image} AS builder"),
        ("Dockerfile", gnu, f"ARG GNU_RUST_IMAGE=rust:{new}@sha256:{digests[new]}"),
        ("Dockerfile", fuzz, f"ENV FUZZ_TOOLCHAIN={new}"),
        (HELPER_PACKAGE, image, f'LINUX_IMAGE = "{new_image}"'),
        (HELPER_PACKAGE, compiler, f'COMPILER = "{new}"'),
    )
    if u.status == "ready":
        u.summary = f"rust toolchain {pinned} → {new}" if new != pinned else f"rustc {pinned} image refresh"
    elif u.status == "current":
        u.note = f"rustc {pinned} and both image digests current"
    return u


def unit_deno(sites: PinSites) -> PinUnit:
    u = PinUnit(key="deno", summary="deno")
    froms = sites.dockerfile_froms()
    dver = sites.dockerfile_arg("DENO_VERSION")
    dsha = sites.dockerfile_arg("DENO_SHA256")
    host = sites.host_deno()
    if not (dver and dsha and host and "denoland/deno" in froms):
        return u.sites_missing()
    hver, hashes = host
    if dver == hver and dsha != hashes["LinuxX86_64Musl"]:
        return u.blocked(
            f"Dockerfile and DENO_RELEASE disagree about the Linux x86_64 asset at v{dver}",
            "pins: deno: coupled asset checksums disagree")
    latest = (github_latest("denoland/deno") or "").lstrip("v")
    if not latest:
        u.status, u.note = "unknown", "deno upstream metadata unreachable this run"
        return u
    tag, pin_digest = froms["denoland/deno"]
    all_sites = {tag, dver, hver}

    if all_sites == {latest}:
        if (cur := docker_hub_digest("denoland/deno", tag)) is None:
            u.status, u.note = "unknown", "deno image digest unreachable this run"
            return u
        if cur == pin_digest:
            u.note = f"deno {latest} current everywhere (Docker images and runtime)"
            return u
        u.status, u.summary = "ready", f"deno {latest} image digest refresh"
        u.note = f"same deno {latest}; the denoland/deno:{tag} tag was republished."
        u.sources = [f"https://registry-1.docker.io/v2/denoland/deno/manifests/{tag} — `sha256:{cur}`"]
        u.edits = [Edit("Dockerfile", f"denoland/deno:{tag}@sha256:{pin_digest}",
                        f"denoland/deno:{tag}@sha256:{cur}")]
        return u

    ahead = max(all_sites, key=vtuple)
    if vtuple(latest) < vtuple(ahead):
        if all_sites == {ahead}:
            u.note = (f"deno {ahead} everywhere, deliberately ahead of upstream latest {latest} "
                      "(the /latest endpoint lags or the pin ran ahead) — nothing to do")
            return u
        return u.blocked(
            f"deno pin sites diverged and upstream latest {latest} is below the highest "
            f"({sorted(all_sites)}) — never auto-downgrade; inspect manually",
            "pins: deno: pin sites diverged above upstream latest")

    new_hashes = {t: deno_sidecar_sha(latest, triple) for t, triple in DENO_TARGETS.items()}
    if missing := [DENO_TARGETS[t] for t, h in new_hashes.items() if not h]:
        return u.blocked(
            f"upstream publishes no .zip.sha256sum sidecar for {missing} at v{latest} — the "
            "platform checksums must be taken manually (download + shasum), never guessed",
            f"pins: deno {latest}: missing asset checksum sidecars")
    if (img := docker_hub_digest("denoland/deno", latest)) is None:
        return u.blocked(f"no denoland/deno:{latest} image tag published yet — wait for the image",
                         f"pins: deno {dver} → {latest} (docker image not published yet)")

    u.summary = f"deno {dver} → {latest}"
    u.note = "Image tag/digest, e2e download and host runtime pin move together."
    if len(all_sites) > 1:
        u.note += f" (sites had diverged: {sorted(all_sites)} — all normalized to {latest}.)"
    u.sources = [
        f"https://api.github.com/repos/denoland/deno/releases/latest — `v{latest}`",
        f"https://registry-1.docker.io/v2/denoland/deno/manifests/{latest} — `sha256:{img}`",
        *[f"https://github.com/denoland/deno/releases/download/v{latest}/deno-{DENO_TARGETS[t]}.zip.sha256sum — "
          f"`{h}`" for t, h in new_hashes.items()],
    ]
    e = u.edits
    e.append(Edit("Dockerfile", f"denoland/deno:{tag}@sha256:{pin_digest}",
                  f"denoland/deno:{latest}@sha256:{img}"))
    e.append(Edit("Dockerfile", f"ARG DENO_VERSION={dver}", f"ARG DENO_VERSION={latest}"))
    e.append(Edit("Dockerfile", f"ARG DENO_SHA256={dsha}", f"ARG DENO_SHA256={new_hashes['LinuxX86_64Musl']}"))
    e.append(Edit(HOST_RS, f'version: "{hver}"', f'version: "{latest}"'))
    for t, old_h in hashes.items():
        e.append(Edit(HOST_RS, old_h, new_hashes[t]))
    if host_comment := re.search(r"Hashes from v[\d.]+", sites.text(HOST_RS)):
        e.append(Edit(HOST_RS, host_comment.group(0), f"Hashes from v{latest}"))
    if f"CI pins {dver}," in sites.text("scripts/check_examples.py"):
        e.append(Edit("scripts/check_examples.py", f"CI pins {dver},", f"CI pins {latest},"))
    u.status = "ready"
    return u


def unit_cargo_auditable(sites: PinSites) -> PinUnit:
    u = PinUnit(key="cargo-auditable", summary="cargo-auditable")
    pinned = sites.dockerfile_arg("CARGO_AUDITABLE_VERSION")
    shas = dict(re.findall(r"(x86_64|aarch64)\)\s+sha=([0-9a-f]{64})", sites.text("Dockerfile")))
    if not pinned or len(shas) != 2:
        return u.sites_missing()
    latest = (github_latest("rust-secure-code/cargo-auditable") or "").lstrip("v")
    if not latest:
        u.status, u.note = "unknown", "cargo-auditable upstream metadata unreachable this run"
        return u
    if vtuple(latest) <= vtuple(pinned):
        u.note = f"cargo-auditable {pinned} current (upstream {latest} not ahead)"
        return u
    assets = github_asset_digests("rust-secure-code/cargo-auditable", f"v{latest}")
    names = {a: f"cargo-auditable-{a}-unknown-linux-musl.tar.xz" for a in shas}
    if not assets or any(not assets.get(n) for n in names.values()):
        return u.blocked(
            f"upstream publishes no API asset digest for {sorted(names.values())} at v{latest} — "
            "the tarball checksums must be taken manually, never guessed",
            f"pins: cargo-auditable {pinned} → {latest}: missing asset digests")
    u.summary = f"cargo-auditable {pinned} → {latest}"
    u.sources = [
        f"https://api.github.com/repos/rust-secure-code/cargo-auditable/releases/latest — `v{latest}`",
        f"https://api.github.com/repos/rust-secure-code/cargo-auditable/releases/tags/v{latest} — "
        "asset `digest` fields (both musl tarballs)",
    ]
    u.edits = [Edit("Dockerfile", f"ARG CARGO_AUDITABLE_VERSION={pinned}",
                    f"ARG CARGO_AUDITABLE_VERSION={latest}")]
    for arch, name in names.items():
        u.edits.append(Edit("Dockerfile", f"sha={shas[arch]}", f"sha={assets[name]}"))
    u.status = "ready"
    return u


def unit_tla(sites: PinSites) -> PinUnit:
    u = PinUnit(key="tla2tools", summary="tla2tools")
    pinned = sites.dockerfile_arg("TLA_TOOLS_VERSION")
    psha = sites.dockerfile_arg("TLA_TOOLS_SHA256")
    if not pinned or not psha:
        return u.sites_missing()
    latest = (github_latest("tlaplus/tlaplus") or "").lstrip("v")
    if not latest:
        u.status, u.note = "unknown", "tlaplus upstream metadata unreachable this run"
        return u
    # the pin has deliberately run ahead of the latest stable tag before;
    # only being BEHIND the latest stable triggers an update.
    if vtuple(latest) <= vtuple(pinned):
        u.note = f"tla2tools {pinned} current (latest stable {latest} not ahead)"
        return u
    assets = github_asset_digests("tlaplus/tlaplus", f"v{latest}")
    if not assets or not assets.get("tla2tools.jar"):
        return u.blocked(f"upstream publishes no API asset digest for tla2tools.jar at v{latest}",
                         f"pins: tla2tools {pinned} → {latest}: missing jar digest")
    u.summary = f"tla2tools {pinned} → {latest}"
    u.sources = [
        f"https://api.github.com/repos/tlaplus/tlaplus/releases/latest — `v{latest}`",
        f"https://api.github.com/repos/tlaplus/tlaplus/releases/tags/v{latest} — tla2tools.jar `digest`",
    ]
    u.edits = [
        Edit("Dockerfile", f"ARG TLA_TOOLS_VERSION={pinned}", f"ARG TLA_TOOLS_VERSION={latest}"),
        Edit("Dockerfile", f"ARG TLA_TOOLS_SHA256={psha}", f"ARG TLA_TOOLS_SHA256={assets['tla2tools.jar']}"),
    ]
    if comment := re.search(r"# tla2tools is checksum-pinned[^\n]*\n#[^\n]*", sites.text("Dockerfile")):
        # the old "deliberately ahead of the latest stable" claim is no
        # longer true after this bump — the prose moves with the pin
        u.edits.append(Edit("Dockerfile", comment.group(0),
                            f"# tla2tools is checksum-pinned to the latest stable tag (v{latest}). "
                            f"Base\n# image digest-pinned like the rest."))
    u.status = "ready"
    return u


def unit_uv(sites: PinSites) -> PinUnit:
    u = PinUnit(key="uv", summary="uv")
    df = sites.text("Dockerfile")
    arg = _line(df, "ARG UV_VERSION=")
    pinned = arg.removeprefix("ARG UV_VERSION=")
    requirement = _line(df, "RUN printf 'uv==")
    # Select the live requirement, not a comment; the Linux install needs
    # just its x86_64 wheel, even when the previous pin allowed two hashes.
    prefix = "RUN printf 'uv==%s"
    suffix = "\\n' \"$UV_VERSION\" > /tmp/uv.req \\"
    hashes = re.escape(prefix) + rf"(?: --hash=sha256:{SHA256}){{1,2}}" + re.escape(suffix)
    install = "    && pip install --no-cache-dir --no-deps --require-hashes -r /tmp/uv.req \\"
    if (not re.fullmatch(VERSION, pinned) or not re.fullmatch(hashes, requirement)
            or requirement + "\n" + install + "\n" not in df):
        return u.sites_missing()
    try:
        latest = fetch_json("https://pypi.org/pypi/uv/json")["info"]["version"]
    except Exception as e:
        print(f"  ! uv: {e}")
        latest = None
    if not isinstance(latest, str) or not re.fullmatch(VERSION, latest):
        u.status, u.note = "unknown", "uv upstream metadata unreachable this run"
        return u
    if vtuple(latest) <= vtuple(pinned):
        u.note = f"uv {pinned} current (upstream {latest} not ahead)"
        return u
    if not (sha := uv_wheel(latest)):
        return u.blocked(
            f"PyPI publishes no unambiguous broadest-manylinux x86_64 wheel digest for uv {latest}",
            f"pins: uv {pinned} → {latest}: missing or ambiguous Linux wheel digest")
    u.summary = f"uv {pinned} → {latest}"
    u.sources = [
        f"https://pypi.org/pypi/uv/json — `{latest}`",
        f"https://pypi.org/pypi/uv/{latest}/json — wheel `digests.sha256` (broadest manylinux x86_64)",
    ]
    return u.propose_lines(sites,
        ("Dockerfile", arg, f"ARG UV_VERSION={latest}"),
        ("Dockerfile", requirement, f"{prefix} --hash=sha256:{sha}{suffix}"),
    )


def unit_node(sites: PinSites) -> PinUnit:
    u = PinUnit(key="node", summary="node")
    df = sites.text("Dockerfile")
    version_arg = _line(df, "ARG LOCAL_NODE_VERSION=")
    sha_arg = _line(df, "ARG LOCAL_NODE_SHA256=")
    pinned = version_arg.removeprefix("ARG LOCAL_NODE_VERSION=")
    download = _line(df, "ADD --checksum=sha256:${LOCAL_NODE_SHA256} ")
    if (not re.fullmatch(VERSION, pinned) or not re.fullmatch(rf"ARG LOCAL_NODE_SHA256={SHA256}", sha_arg)
            or download != "ADD --checksum=sha256:${LOCAL_NODE_SHA256} https://nodejs.org/dist/"
                           "v${LOCAL_NODE_VERSION}/node-v${LOCAL_NODE_VERSION}-linux-x64.tar.xz /tmp/node.tar.xz"):
        return u.sites_missing()
    major = pinned.split(".")[0]
    dist = "https://nodejs.org/dist"
    try:
        index = fetch_json(f"{dist}/index.json")
        latest = max((row["version"][1:] for row in index
                      if re.fullmatch(rf"v{major}\.[0-9]+\.[0-9]+", row["version"])), key=vtuple)
    except Exception as e:
        print(f"  ! node: {e}")
        u.status, u.note = "unknown", f"no stable Node {major} in the dist index this run"
        return u
    if vtuple(latest) <= vtuple(pinned):
        u.note = f"node {pinned} current (upstream {latest} not ahead)"
        return u
    url = f"{dist}/v{latest}/SHASUMS256.txt"
    artifact = f"node-v{latest}-linux-x64.tar.xz"
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=30) as response:
            rows = [line.split() for line in response.read().decode("ascii").splitlines()]
        matches = [row for row in rows if row and row[-1] == artifact]
    except Exception as e:
        print(f"  ! {url}: {e}")
        matches = []
    if len(matches) != 1 or len(matches[0]) != 2 or not re.fullmatch(SHA256, matches[0][0]):
        return u.blocked(f"No unique valid checksum for {artifact} in {url}",
                         f"pins: node {pinned} → {latest}: missing or ambiguous Linux checksum")
    sha = matches[0][0]
    u.summary = f"node {pinned} → {latest}"
    u.sources = [f"{dist}/index.json — `{latest}` (stable major {major})", f"{url} — `{sha}  {artifact}`"]
    return u.propose_lines(sites,
        ("Dockerfile", version_arg, f"ARG LOCAL_NODE_VERSION={latest}"),
        ("Dockerfile", sha_arg, f"ARG LOCAL_NODE_SHA256={sha}"),
    )


def unit_base_images(sites: PinSites) -> list[PinUnit]:
    """Digest-only FROM pins coupled to nothing else (temurin, python…)."""
    out = []
    for name, (tag, pin_digest) in sites.dockerfile_froms().items():
        if name in ("rust", "denoland/deno"):
            continue  # owned by the coupled rust/deno units
        u = PinUnit(key=f"img-{name}-{tag}".replace(":", "-").replace("/", "-"),
                    summary=f"base image {name}:{tag} digest refresh")
        repository = name if "/" in name else f"library/{name}"
        cur = docker_hub_digest(repository, tag)
        if cur is None:
            u.status, u.note = "unknown", f"{name}:{tag} digest unreachable this run"
        elif cur == pin_digest:
            u.note = f"{name}:{tag} digest current"
        else:
            u.status = "ready"
            u.sources = [f"https://registry-1.docker.io/v2/{repository}/manifests/{tag} — `sha256:{cur}`"]
            u.edits = [Edit("Dockerfile", f"{name}:{tag}@sha256:{pin_digest}",
                            f"{name}:{tag}@sha256:{cur}")]
        out.append(u)
    return out


def build_units(sites: PinSites) -> list[PinUnit]:
    return [unit_rust(sites), unit_deno(sites), unit_cargo_auditable(sites),
            unit_tla(sites), unit_uv(sites), unit_node(sites), *unit_base_images(sites)]


# --- report (dry-run) and authorized application --------------------------

def render(units: list[PinUnit], root: str) -> None:
    """Per-unit status + the exact diffs a PR would contain. Never writes."""
    marks = {"current": "✓", "ready": "+", "blocked": "✗", "unknown": "!"}
    for u in units:
        print(f"\n== {u.key}: {u.summary or u.key} [{marks[u.status]} {u.status}]")
        if u.note:
            print(f"   {u.note}")
        if u.manual:
            print("   (--apply opens a manual-work issue; no PR until the data exists)")
        by_path: dict[str, list[Edit]] = {}
        for e in u.edits:
            by_path.setdefault(e.path, []).append(e)
        for path, es in by_path.items():
            try:
                with open(os.path.join(root, path)) as f:
                    old_text = f.read()
            except OSError as e:
                print(f"   ! cannot read {path}: {e}")
                continue
            new_text = old_text
            for e in es:
                if new_text.count(e.old) != e.count:
                    print(f"   ! {path}: edit site drifted ({e.old[:40]!r} not found {e.count}×) — would fail closed")
                    new_text = None
                    break
                new_text = new_text.replace(e.old, e.new)
            if new_text and new_text != old_text:
                sys.stdout.writelines(difflib.unified_diff(
                    old_text.splitlines(keepends=True), new_text.splitlines(keepends=True),
                    fromfile=path, tofile=f"{path} (proposed)"))


def open_issue(title: str, body: str) -> None:
    existing = subprocess.run(
        ["gh", "issue", "list", "-R", REPO, "--search", f"in:title {title}", "--state", "open", "--json", "number"],
        capture_output=True, text=True,
    ).stdout
    if json.loads(existing or "[]"):
        print(f"  = issue already open: {title}")
        return
    subprocess.run(
        ["gh", "issue", "create", "-R", REPO, "--title", title,
         "--label", "help wanted", "--label", "needs-triage", "--body", body],
        check=True,
    )
    print(f"  + opened issue: {title}")


def apply_units(units: list[PinUnit]) -> int:
    import shutil
    import pin_pr

    if not (shutil.which("gh") and shutil.which("git")):
        print("error: --apply needs gh and git on PATH (authorized CI)", file=sys.stderr)
        return 2
    if not (os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")):
        print("error: --apply needs GH_TOKEN/GITHUB_TOKEN (authorized CI)", file=sys.stderr)
        return 2
    builders = {"rust": unit_rust, "deno": unit_deno, "cargo-auditable": unit_cargo_auditable,
                "tla2tools": unit_tla, "uv": unit_uv, "node": unit_node}

    def rebuild(key: str):
        if key in builders:
            return lambda root: builders[key](PinSites(root))
        return lambda root: next(u for u in unit_base_images(PinSites(root)) if u.key == key)

    failures = 0
    for u in units:
        if u.status == "ready":
            try:
                print(f"\n== {u.key}: publishing pins/{u.key}")
                print(f"  + {u.key}: {pin_pr.publish(u.key, rebuild(u.key))}")
            except Exception as e:
                failures += 1
                print(f"  ! {u.key}: {e}")
        elif u.status == "blocked" and u.manual:
            open_issue(u.issue_title, u.manual)
    if failures:
        print(f"\n{failures} unit(s) failed to publish — see above", file=sys.stderr)
        return 3
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--dry-run", action="store_true",
                   help="read-only: report drift and the exact diffs a PR would contain (default)")
    g.add_argument("--apply", action="store_true",
                   help="authorized CI: push pins/* branches, open/reuse PRs, dispatch gates, issue fail-closed pins")
    ap.add_argument("--units", default=None,
                    help="comma-separated unit keys to consider (rust,deno,tla2tools,uv,node,cargo-auditable,img-*)")
    args = ap.parse_args(argv)

    units = build_units(PinSites())
    if args.units:
        wanted = {k.strip() for k in args.units.split(",")}
        if missing := wanted - {u.key for u in units}:
            ap.error(f"unknown unit(s): {sorted(missing)}")
        units = [u for u in units if u.key in wanted]

    print(f"== pin proposal report ({'apply' if args.apply else 'dry-run'})")
    render(units, ".")
    if not args.apply:
        print("\n(dry-run: nothing written, no git/gh calls)")
        return 0
    return apply_units(units)


if __name__ == "__main__":
    sys.exit(main())
