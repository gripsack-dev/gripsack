#!/usr/bin/env python3
"""Pin freshness watch + deliberate pin-update PR proposals (plan/0042 F).

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
             to pins/<name>, the PR opened/reused by head branch, and
             ci+repro dispatched on the branch — GITHUB_TOKEN PRs do not
             trigger CI by themselves, but those workflows already
             declare workflow_dispatch, so the dispatch needs only
             actions:write. Pins that cannot move yet fail closed as
             manual-work issues. Idempotent.

Coupled pins move as ONE proposal each:
  rust    Dockerfile rust:alpine digest + FUZZ_TOOLCHAIN + the native
          dtolnay/rust-toolchain pins (ci.yml, release-core.yml). The
          image's rustc must equal the native pin — proven by digest
          equality with the rust:<version>-alpine tag (identical
          manifests are identical images), not by running the image.
  deno    Dockerfile (image digest, DENO_VERSION, DENO_SHA256), host.rs
          DENO_RELEASE (version + every platform hash), the workflows'
          deno-version pins, and ci.yml's macOS prefetch URL+sha.
  tools   cargo-auditable (version + both musl tarball sha256s), tla2tools
          (version + jar sha256), uv (version + both wheel hashes),
          node 22 (both setup-node pins).
  images  other digest-pinned FROM lines (eclipse-temurin, python) —
          tag-scoped digests coupled to nothing else.

Metadata sources (official, read-only): Docker Hub registry API
(manifest digests), static.rust-lang.org stable channel, GitHub
releases API (latest tags + asset sha256 digests), deno release
.zip.sha256sum sidecars, PyPI JSON (uv wheels), nodejs.org dist index.

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

WORKFLOW_FILES = [
    ".github/workflows/ci.yml",
    ".github/workflows/release-core.yml",
    ".github/workflows/release-typescript.yml",
    ".github/workflows/examples.yml",
]
HOST_RS = "crates/gripsack-fetch/src/host.rs"

# deno asset name per host.rs AssetTarget slot (glibc builds for the
# linux slots — deno ships no musl build, host.rs maps them)
DENO_TARGETS = {
    "LinuxX86_64Musl": "x86_64-unknown-linux-gnu",
    "LinuxAarch64Musl": "aarch64-unknown-linux-gnu",
    "MacosX86_64": "x86_64-apple-darwin",
    "MacosAarch64": "aarch64-apple-darwin",
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
            return r.headers.get("Docker-Content-Digest", "").removeprefix("sha256:")
    except Exception as e:
        print(f"  ! {repository}:{tag}: {e}")
        return None


def channel_stable() -> tuple[str, str] | None:
    """(short, full) for current stable rustc, e.g. ("1.98.1", "1.98.1 (48a… 2026-09-01)")."""
    try:
        toml = urllib.request.urlopen(
            "https://static.rust-lang.org/dist/channel-rust-stable.toml", timeout=30).read().decode()
        full = re.search(r'\[pkg\.rust\]\nversion = "([^"]+)"', toml).group(1)
        return full.split(" ", 1)[0], full
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


def uv_wheels(version: str) -> tuple[str, str] | None:
    """(x86_64, aarch64) sha256 for the broadest-manylinux uv wheel of
    each arch — the same choice the Dockerfile pin encodes today. Zero
    or ambiguous candidates per arch -> None (manual work)."""
    try:
        urls = fetch_json(f"https://pypi.org/pypi/uv/{version}/json")["urls"]
    except Exception as e:
        print(f"  ! uv {version}: {e}")
        return None

    def pick(arch: str) -> str | None:
        floors = []
        for f in urls:
            fn = f["filename"]
            if not (fn.startswith(f"uv-{version}-py3-none-") and "manylinux" in fn
                    and fn.endswith(f"{arch}.whl")):
                continue
            g = re.findall(r"manylinux_2_(\d+)", fn)
            floors.append((min(float(x) for x in g) if g else 17.0, f["digests"]["sha256"]))
        if not floors:
            return None
        lowest = min(f for f, _ in floors)
        winners = {sha for f, sha in floors if f == lowest}
        return winners.pop() if len(winners) == 1 else None

    x86, arm = pick("x86_64"), pick("aarch64")
    return (x86, arm) if x86 and arm else None


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
            self._files[path] = open(os.path.join(self.root, path)).read()
        return self._files[path]

    def dockerfile_arg(self, name: str) -> str | None:
        m = re.search(rf"^ARG {re.escape(name)}=(\S+)", self.text("Dockerfile"), re.M)
        return m.group(1) if m else None

    def dockerfile_env(self, name: str) -> str | None:
        m = re.search(rf"^ENV {re.escape(name)}=(\S+)", self.text("Dockerfile"), re.M)
        return m.group(1) if m else None

    def dockerfile_froms(self) -> dict[str, tuple[str, str]]:
        """name -> (tag, digest) for every digest-pinned FROM line."""
        out = {}
        for ref, dig in re.findall(r"^FROM (\S+)@sha256:([0-9a-f]{64})", self.text("Dockerfile"), re.M):
            name, tag = ref.rsplit(":", 1)
            out[name] = (tag, dig)
        return out

    def workflow_pins(self, key: str) -> dict[str, list[str]]:
        """workflow file -> the pin values under `key:` (raw value minus
        an optional quote; the deno `v` prefix stays in the value)."""
        out = {}
        for p in WORKFLOW_FILES:
            vals = re.findall(rf"^\s*{re.escape(key)}:\s*[\"']?([\w.+-]+)", self.text(p), re.M)
            if vals:
                out[p] = vals
        return out

    def host_deno(self) -> tuple[str, dict[str, str]] | None:
        """(version, {AssetTarget: sha256}) from host.rs DENO_RELEASE."""
        src = self.text(HOST_RS)
        m = re.search(r"pub const DENO_RELEASE: ToolRelease = ToolRelease \{(.*?)\n\};", src, re.S)
        if not m:
            return None
        ver = re.search(r'version:\s*"([^"]+)"', m.group(1))
        pairs = re.findall(r"AssetTarget::(\w+),\s*\n?\s*\"([0-9a-f]{64})\"", m.group(1))
        if not ver or {t for t, _ in pairs} != set(DENO_TARGETS):
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

    def pr_body(self) -> str:
        lines = [
            f"### pins: {self.summary}",
            "",
            "Deliberate pin update (plan/0042 F), prepared by `scripts/check_pins.py`",
            "(upstream-watch) from official upstream metadata only. Never a silent",
            "floating upgrade, never auto-merged — review and merge deliberately.",
            "",
        ]
        lines += [
            "**Review checklist**",
            "- [ ] the sources above still resolve to these exact values",
            "- [ ] coupled pin sites all moved together in this PR",
            "- [ ] `scripts/check_reproducible.sh` passes on this branch (the repro",
            "      gate is dispatched on it — see the run links below)",
            "",
            "Gates dispatched on this branch by the proposer: `ci.yml`, `repro.yml`.",
        ]
        return "\n".join(lines)


MANUAL_WORK = (
    "This pin cannot move automatically yet — the pin-update automation "
    "(scripts/check_pins.py, plan/0042 F) fails closed here rather than "
    "fabricate a checksum or decouple pins that must move together. "
    "Update deliberately per plan/0042 F: verify from official metadata "
    "(digest/manifest check, `rustc -vV`, asset sha256), move every "
    "coupled pin site in one reviewed PR, run "
    "`scripts/check_reproducible.sh` before and after.")


def unit_rust(sites: PinSites) -> PinUnit:
    u = PinUnit(key="rust", summary="rust toolchain")
    froms = sites.dockerfile_froms()
    native = sites.workflow_pins("toolchain")
    fuzz = sites.dockerfile_env("FUZZ_TOOLCHAIN")
    if "rust" not in froms or not native or fuzz is None:
        return u.sites_missing()
    tag, pin_digest = froms["rust"]
    pinned = next(iter(native.values()))[0]
    stable = channel_stable()
    cur = docker_hub_digest("library/rust", tag)
    if stable is None or cur is None:
        u.status, u.note = "unknown", "stable channel / Docker Hub metadata unreachable this run"
        return u
    u.sources = [
        f"https://static.rust-lang.org/dist/channel-rust-stable.toml — stable `{stable[1]}`",
        f"https://registry-1.docker.io/v2/library/rust/manifests/{tag} — `sha256:{cur}`",
    ]
    native_vals = {v for vs in native.values() for v in vs}

    def diverged() -> PinUnit | None:
        """The coupled pins disagree among themselves with no bump to
        ride on — that never silently passes; only the full coupled
        bump normalizes it."""
        if fuzz == pinned and native_vals == {pinned}:
            return None
        return u.blocked(
            f"rust pins diverged without an upstream bump (native {sorted(native_vals)}, "
            f"FUZZ_TOOLCHAIN {fuzz}, pinned base {pinned}) — only a deliberate coupled PR fixes this",
            "pins: rust: coupled pins diverged without an upstream bump")

    def image_lagging(why: str) -> PinUnit:
        return u.blocked(
            f"stable rustc is now {stable[0]} but the digest-pinned rust:{tag} still ships "
            f"{pinned} ({why}) — a native-only bump would silently decouple the docker builder "
            "from the native jobs; no PR until the image catches up, then the next watch run "
            "opens the coupled one automatically",
            f"pins: rust toolchain {pinned} behind upstream {stable[0]} (image lagging)")

    if vtuple(stable[0]) <= vtuple(pinned):
        if d := diverged():
            return d
        if cur == pin_digest:
            u.note = f"rustc {pinned} current (stable {stable[0]} not ahead)"
            return u
        # tag moved under the same rustc: a rebuild. Prove the floating
        # tag still resolves to OUR version's image via the version tag.
        if docker_hub_digest("library/rust", f"{pinned}-{tag}") == cur:
            u.status, u.summary = "ready", f"rust:alpine digest refresh (rustc {pinned} unchanged)"
            u.edits = [Edit("Dockerfile", f"rust:{tag}@sha256:{pin_digest}", f"rust:{tag}@sha256:{cur}")]
            return u
        return u.blocked(
            f"rust:{tag} moved to a build that is not {pinned} "
            f"(digest(rust:{pinned}-{tag}) != digest(rust:{tag})) — inspect manually",
            f"pins: rust: {tag} tag moved off rustc {pinned}")

    # stable is ahead — the native pins, FUZZ_TOOLCHAIN and the image
    # digest must move together or the docker/native builds decouple.
    if cur == pin_digest:
        return image_lagging("the floating tag has not moved off our digest")
    ver_digest = docker_hub_digest("library/rust", f"{stable[0]}-{tag}")
    if ver_digest is None:
        return image_lagging(f"no rust:{stable[0]}-{tag} image tag exists yet")
    if ver_digest != cur:
        return image_lagging(f"digest(rust:{tag}) != digest(rust:{stable[0]}-{tag})")

    # coupling proven: the floating tag IS the new stable's image
    new, full = stable
    u.sources.append(f"digest(rust:{tag}) == digest(rust:{new}-{tag}) — identical manifests, "
                     "so the image's rustc IS the new native pin")
    u.summary = f"rust toolchain {pinned} → {new}"
    u.note = f"docker builder digest, FUZZ_TOOLCHAIN and native pins move together to {new}."
    if fuzz != pinned:
        u.note += f" (FUZZ_TOOLCHAIN was {fuzz} — normalized to {new}.)"
    if native_vals != {pinned}:
        u.note += " (native toolchain pins had diverged — normalized.)"
    u.edits = [Edit("Dockerfile", f"rust:{tag}@sha256:{pin_digest}", f"rust:{tag}@sha256:{cur}")]
    df = sites.text("Dockerfile")
    if old_full := re.search(rf"{re.escape(pinned)} \([0-9a-f]+ \d{{4}}-\d{{2}}-\d{{2}}\)", df):
        u.edits.append(Edit("Dockerfile", old_full.group(0), full))
    u.edits.append(Edit("Dockerfile", f"ENV FUZZ_TOOLCHAIN={fuzz}", f"ENV FUZZ_TOOLCHAIN={new}"))
    for path, vals in native.items():
        for old in dict.fromkeys(vals):
            u.edits.append(Edit(path, f'toolchain: "{old}"', f'toolchain: "{new}"', vals.count(old)))
    ci = sites.text(".github/workflows/ci.yml")
    if f"— {pinned}" in ci:
        u.edits.append(Edit(".github/workflows/ci.yml", f"— {pinned}", f"— {new}"))
    if old_paren := re.search(r"\([0-9a-f]+ \d{4}-\d{2}-\d{2}\)", ci):
        u.edits.append(Edit(".github/workflows/ci.yml", old_paren.group(0),
                            re.search(r"\([^)]+\)$", full).group(0)))
    rc = sites.text(".github/workflows/release-core.yml")
    if f"({pinned})" in rc:
        u.edits.append(Edit(".github/workflows/release-core.yml", f"({pinned})", f"({new})"))
    u.status = "ready"
    return u


def unit_deno(sites: PinSites) -> PinUnit:
    u = PinUnit(key="deno", summary="deno")
    froms = sites.dockerfile_froms()
    dver = sites.dockerfile_arg("DENO_VERSION")
    dsha = sites.dockerfile_arg("DENO_SHA256")
    host = sites.host_deno()
    wf = sites.workflow_pins("deno-version")
    if not (dver and dsha and host and "denoland/deno" in froms and wf):
        return u.sites_missing()
    hver, hashes = host
    latest = (github_latest("denoland/deno") or "").lstrip("v")
    if not latest:
        u.status, u.note = "unknown", "deno upstream metadata unreachable this run"
        return u
    tag, pin_digest = froms["denoland/deno"]
    all_sites = {dver, hver, *(v.lstrip("v") for vs in wf.values() for v in vs)}

    if all_sites == {latest}:
        if (cur := docker_hub_digest("denoland/deno", tag)) is None:
            u.status, u.note = "unknown", "deno image digest unreachable this run"
            return u
        if cur == pin_digest:
            u.note = f"deno {latest} current everywhere (docker/runtime/workflows)"
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
    u.note = "image digest, Dockerfile pins, host.rs runtime table and workflow pins move together."
    if len(all_sites) > 1:
        u.note += f" (sites had diverged: {sorted(all_sites)} — all normalized to {latest}.)"
    u.sources = [
        f"https://api.github.com/repos/denoland/deno/releases/latest — `v{latest}`",
        f"https://registry-1.docker.io/v2/denoland/deno/manifests/{latest} — `sha256:{img}`",
        *[f"https://github.com/denoland/deno/releases/download/v{latest}/deno-{t}.zip.sha256sum — "
          f"`{h[:16]}…`" for t, h in new_hashes.items()],
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
    ci = sites.text(".github/workflows/ci.yml")
    if ci_url := re.search(r"releases/download/v[\d.]+/deno-aarch64-apple-darwin\.zip", ci):
        e.append(Edit(".github/workflows/ci.yml", ci_url.group(0),
                      f"releases/download/v{latest}/deno-aarch64-apple-darwin.zip"))
    if ci_sha := re.search(r"[0-9a-f]{64}  /tmp/deno\.zip", ci):
        e.append(Edit(".github/workflows/ci.yml", ci_sha.group(0),
                      f"{new_hashes['MacosAarch64']}  /tmp/deno.zip"))
    for path, vals in wf.items():
        for old in dict.fromkeys(vals):
            e.append(Edit(path, f"deno-version: {old}", f"deno-version: v{latest}", vals.count(old)))
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
    pinned = sites.dockerfile_arg("UV_VERSION")
    m = re.search(r"--hash=sha256:([0-9a-f]{64}) --hash=sha256:([0-9a-f]{64})", sites.text("Dockerfile"))
    if not pinned or not m:
        return u.sites_missing()
    try:
        latest = fetch_json("https://pypi.org/pypi/uv/json")["info"]["version"]
    except Exception as e:
        print(f"  ! uv: {e}")
        latest = None
    if not latest:
        u.status, u.note = "unknown", "uv upstream metadata unreachable this run"
        return u
    if vtuple(latest) <= vtuple(pinned):
        u.note = f"uv {pinned} current (upstream {latest} not ahead)"
        return u
    if not (wheels := uv_wheels(latest)):
        return u.blocked(
            f"PyPI publishes no unambiguous broadest-manylinux wheel pair for uv {latest} — the "
            "pip --require-hashes digests must be taken manually, never guessed",
            f"pins: uv {pinned} → {latest}: ambiguous wheel digests")
    u.summary = f"uv {pinned} → {latest}"
    u.sources = [
        f"https://pypi.org/pypi/uv/json — `{latest}`",
        f"https://pypi.org/pypi/uv/{latest}/json — wheel `digests.sha256` (broadest manylinux per arch)",
    ]
    u.edits = [
        Edit("Dockerfile", f"ARG UV_VERSION={pinned}", f"ARG UV_VERSION={latest}"),
        Edit("Dockerfile", f"--hash=sha256:{m.group(1)}", f"--hash=sha256:{wheels[0]}"),
        Edit("Dockerfile", f"--hash=sha256:{m.group(2)}", f"--hash=sha256:{wheels[1]}"),
    ]
    u.status = "ready"
    return u


def unit_node(sites: PinSites) -> PinUnit:
    u = PinUnit(key="node", summary="node 22")
    pins = sites.workflow_pins("node-version")
    if not pins:
        return u.sites_missing()
    try:
        idx = fetch_json("https://nodejs.org/dist/index.json")
        latest = next(e["version"].lstrip("v") for e in idx if e["version"].startswith("v22."))
    except Exception as e:
        print(f"  ! node: {e}")
        latest = None
    if not latest:
        u.status, u.note = "unknown", "node dist index unreachable this run"
        return u
    vals = [v for vs in pins.values() for v in vs]
    if all(v == latest for v in vals):
        u.note = f"node {latest} current in every setup-node pin"
        return u
    ahead = max(vals, key=vtuple)
    if vtuple(latest) < vtuple(ahead):
        if set(vals) == {ahead}:
            u.note = (f"node {ahead} pinned everywhere, deliberately ahead of the dist index "
                      f"({latest}) — nothing to do")
            return u
        return u.blocked(
            f"node-version pins diverged and the dist index ({latest}) is below the highest "
            f"({sorted(set(vals))}) — never auto-downgrade",
            "pins: node: pin sites diverged above the dist index")
    u.summary = f"node 22 {vals[0]} → {latest}"
    u.sources = [f"https://nodejs.org/dist/index.json — `{latest}` (first v22 entry)"]
    for path, vs in pins.items():
        for old in dict.fromkeys(vs):
            u.edits.append(Edit(path, f"node-version: {old}", f"node-version: {latest}", vs.count(old)))
    u.status = "ready"
    return u


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
         "--label", "help wanted", "--body", body],
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
