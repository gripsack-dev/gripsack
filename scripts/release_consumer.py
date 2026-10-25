#!/usr/bin/env python3
"""Fail-closed release admission shared by installer and embedded self-update.

The GitHub CLI and cosign are independently provisioned trust dependencies
(hosted attestation and pinned owner key respectively), never bootstrapped
from the candidate. Fixtures may replace its process boundary, but cannot serve
as evidence of cryptographic verification. No downloaded program is executed.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import stat
import tarfile
import tempfile
import time

import release_manifest as manifest_schema

REPOSITORY = "gripsack-dev/gripsack"
API = f"https://api.github.com/repos/{REPOSITORY}"
WORKFLOW = ".github/workflows/release-core.yml"
PREDICATE = "https://gripsack.dev/attestations/local-qualification/v1"
LOCAL_SIGNATURE_SUFFIX = ".local-signature.json"
OWNER_KEY_ENV = "GRIPSACK_OWNER_SIGNER_KEY"
MAX_OWNER_KEY_BYTES = manifest_schema.MAX_PUBLIC_KEY_BYTES
TARGET = "x86_64-unknown-linux-musl"
MAX_JSON = 8 * 1024 * 1024
MAX_ARTIFACT = 1024 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def command(args, *, timeout=300, maximum=MAX_JSON):
    process = subprocess.Popen(
        args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        start_new_session=True, env=dict(os.environ, GH_PROMPT_DISABLED="1", GH_PAGER="cat"),
    )
    selector = selectors.DefaultSelector()
    output, errors = bytearray(), bytearray()
    deadline = time.monotonic() + timeout
    try:
        selector.register(process.stdout, selectors.EVENT_READ, output)
        selector.register(process.stderr, selectors.EVENT_READ, errors)
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"{args[0]} exceeded its verification deadline")
            for key, _ in selector.select(remaining):
                data = os.read(key.fileobj.fileno(), 65536)
                if not data:
                    selector.unregister(key.fileobj)
                    continue
                require(len(output) + len(errors) + len(data) <= maximum,
                        f"{args[0]} output exceeds verification bound")
                key.data.extend(data)
        process.wait(timeout=max(0.001, deadline - time.monotonic()))
        require(process.returncode == 0,
                f"{args[0]} failed: {errors[-4096:].decode('utf-8', errors='replace')}")
        return output.decode("utf-8")
    finally:
        selector.close()
        # Until wait() reaps it, the leader reserves this process-group ID.
        # Never signal a possibly reused group after a known terminal result.
        if process.returncode is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        process.stdout.close()
        process.stderr.close()


def verifier():
    require(shutil.which("gh") is not None,
            "GitHub CLI >= 2.102.0 with attestation verify is required; no unsigned fallback")
    version = command(["gh", "--version"])
    match = re.search(r"gh version (\d+)\.(\d+)\.(\d+)", version)
    require(match is not None and tuple(map(int, match.groups())) >= (2, 102, 0),
            "GitHub CLI >= 2.102.0 is required (patched attestation identity and download policy)")
    help_text = command(["gh", "attestation", "verify", "--help"])
    for flag in ("--source-ref", "--source-digest", "--signer-digest", "--cert-identity",
                 "--predicate-type", "--deny-self-hosted-runners"):
        require(flag in help_text, f"GitHub CLI verifier lacks required capability {flag}")

def local_verifier():
    require(shutil.which("cosign") is not None,
            "cosign >= 2.0.0 with offline key verification is required; no unsigned fallback")
    version = command(["cosign", "version"])
    match = re.search(r"v?(\d+)\.(\d+)\.(\d+)", version)
    require(match is not None and tuple(map(int, match.groups())) >= (2, 0, 0),
            "cosign >= 2.0.0 is required for offline owner-key verification")
    help_text = command(["cosign", "verify-blob", "--help"])
    for flag in ("--key", "--signature", "--insecure-ignore-tlog"):
        require(flag in help_text, f"cosign verifier lacks required capability {flag}")


def pinned_owner_key(owner_key=None):
    """The pinned owner public key is an independently provisioned trust root.

    Like the GitHub CLI, cosign is provisioned by the verifying host, never
    bootstrapped from the candidate release. The pinned key path must be
    absolute; the private half exists only on the dedicated signer host.
    """
    if owner_key is None:
        named = os.environ.get(OWNER_KEY_ENV, "")
        require(named, f"{OWNER_KEY_ENV} must name the pinned owner public key; no digest-only fallback")
        path = Path(named)
        require(path.is_absolute(), "pinned owner public key path must be absolute")
        path = manifest_schema.regular(path)
        with path.open("rb") as stream:
            owner_key = stream.read(MAX_OWNER_KEY_BYTES + 1).decode("ascii")
    require(isinstance(owner_key, str) and len(owner_key.encode()) <= MAX_OWNER_KEY_BYTES,
            "pinned owner public key exceeds bound")
    return owner_key.strip()


def verify_local_signature(path: Path, manifest, signature: Path, *, command_runner=None,
                           owner_key=None, scratch=None):
    """Full owner-key authority: pin check, identity binding, then cryptography.

    A wrong key, a foreign authority or any unbound identity field refuses
    before the cryptographic boundary; no checksum ever substitutes. With a
    ``scratch`` directory the cosign invocation is byte-stable, so retained
    evidence can replay the exact recorded command.
    """
    pinned = pinned_owner_key(owner_key)
    fingerprint = manifest_schema.key_fingerprint(pinned)
    document = manifest_schema.validate_signature_document(
        manifest_schema.load_json(manifest_schema.regular(signature)))
    require(document["public_key"] == pinned
            and document["payload"]["key_fingerprint"] == fingerprint,
            "local signature is not by the pinned owner key")
    expected = manifest_schema.signature_payload(manifest, manifest_schema.digest(path), fingerprint)
    require(document["payload"] == expected,
            "local signature does not bind the admitted release identity")
    runner = command if command_runner is None else command_runner
    if scratch is None:
        temporary = tempfile.TemporaryDirectory(prefix="gripsack-local-signature-")
        root = Path(temporary.name)
    else:
        temporary = None
        root = Path(scratch)
        root.mkdir(mode=0o700, parents=True, exist_ok=True)
    try:
        (root / "payload.json").write_bytes(manifest_schema.canonical(document["payload"]))
        (root / "owner.pub").write_text(pinned + "\n")
        (root / "signature.b64").write_text(document["signature"] + "\n")
        output = runner(["cosign", "verify-blob", "--key", str(root / "owner.pub"),
                         "--signature", str(root / "signature.b64"), "--insecure-ignore-tlog",
                         str(root / "payload.json")])
        require("Verified OK" in output, "owner key signature verification failed")
    finally:
        if temporary is not None:
            temporary.cleanup()
    return document


def platform_check():
    require(command(["uname", "-s"]).strip() == "Linux", "only Linux/WSL is supported")
    require(command(["uname", "-m"]).strip() in {"x86_64", "amd64"},
            "only Linux x86_64 is qualified")


def download(url: str, path: Path, maximum: int):
    require(url.startswith("https://"), "download requires HTTPS")
    command(["curl", "--fail", "--silent", "--show-error", "--location",
             "--proto", "=https", "--proto-redir", "=https", "--max-time", "300",
             "--max-filesize", str(maximum), "--output", str(path), url])
    require(path.is_file() and path.stat().st_size <= maximum, "download exceeds size bound")


def fetch_json(url: str):
    with tempfile.TemporaryDirectory(prefix="gripsack-api-") as directory:
        path = Path(directory) / "response.json"
        download(url, path, MAX_JSON)
        return manifest_schema.load_json(path)


def stable_version(tag):
    match = re.fullmatch(r"core-v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", tag)
    return tuple(map(int, match.groups())) if match else None


def select_release(version=None):
    if version is not None:
        require(stable_version(f"core-v{version}") is not None, "expected stable core version")
        release = fetch_json(f"{API}/releases/tags/core-v{version}")
    else:
        releases = []
        for page in range(1, 11):
            batch = fetch_json(f"{API}/releases?per_page=100&page={page}")
            require(isinstance(batch, list), "invalid release catalog")
            releases.extend(item for item in batch if stable_version(item.get("tag_name", ""))
                            and item.get("draft") is False and item.get("prerelease") is False)
            if len(batch) < 100:
                break
        else:
            raise ValueError("release catalog exceeds bound; select an explicit version")
        require(bool(releases), "no published stable release; unsigned historical releases are not admitted")
        release = max(releases, key=lambda item: stable_version(item["tag_name"]))
    require(release.get("draft") is False and release.get("prerelease") is False
            and stable_version(release.get("tag_name", "")) is not None,
            "release is withdrawn, draft, prerelease or unsupported")
    return release


def release_assets(release):
    identity = release.get("id")
    require(type(identity) is int and identity > 0, "release has no immutable API identity")
    assets = []
    for page in range(1, 11):
        batch = fetch_json(f"{API}/releases/{identity}/assets?per_page=100&page={page}")
        require(isinstance(batch, list), "invalid release asset inventory")
        assets.extend(batch)
        if len(batch) < 100:
            break
    else:
        raise ValueError("release asset inventory exceeds bound")
    names = [item.get("name") for item in assets]
    require(len(names) == len(set(names)), "duplicate public asset name")
    return {item["name"]: item for item in assets}


def tag_source(tag):
    obj = fetch_json(f"{API}/git/ref/tags/{tag}")["object"]
    for _ in range(4):
        require(re.fullmatch(r"[0-9a-f]{40}", obj.get("sha", "")) is not None,
                "invalid source object")
        if obj["type"] == "commit":
            commit = fetch_json(f"{API}/git/commits/{obj['sha']}")
            return {"commit": obj["sha"], "tree": commit["tree"]["sha"]}
        require(obj["type"] == "tag", "tag does not resolve to a source commit")
        obj = fetch_json(f"{API}/git/tags/{obj['sha']}")["object"]
    raise ValueError("tag indirection exceeds bound")


def expected_predicate(manifest):
    return {"format": "gripsack.local-qualification.v1", "policy": manifest["policy"],
            "source": manifest["source"], "candidate_id": manifest["candidate_id"],
            "qualification_sha256": manifest["qualification_sha256"],
            "recipe_sha256": manifest["recipe_sha256"],
            "builder": "local", "hosted_build": False}


def verify_attestation(path: Path, manifest, bundle: Path, *, command_runner=None):
    ref = manifest["signer"]["ref"]
    source = manifest["source"]["commit"]
    runner = command if command_runner is None else command_runner
    args = ["gh", "attestation", "verify", str(path), "--repo", REPOSITORY,
            "--hostname", "github.com", "--cert-identity",
            f"https://github.com/{REPOSITORY}/{WORKFLOW}@{ref}",
            "--cert-oidc-issuer", "https://token.actions.githubusercontent.com",
            "--source-ref", ref, "--source-digest", source, "--signer-digest", source,
            "--predicate-type", PREDICATE, "--deny-self-hosted-runners", "--format", "json"]
    # Verify the published bundle, then the API association. The latter protects
    # against reuse of an old repository name by a different repository.
    for suffix in (["--bundle", str(bundle)], []):
        results = json.loads(runner(args + suffix))
        require(isinstance(results, list) and bool(results), "no verified attestation")
        wanted = expected_predicate(manifest)
        matching = []
        for result in results:
            statement = result.get("verificationResult", {}).get("statement", {})
            subjects = statement.get("subject", [])
            matching.append(statement.get("predicateType") == PREDICATE
                            and statement.get("predicate") == wanted
                            and any(subject.get("digest") == {"sha256": manifest_schema.digest(path)}
                                    for subject in subjects))
        require(any(matching), "verified signature does not bind required local qualification policy")


def extract_core(archive: Path, version: str, destination: Path):
    root = f"gripsack-{version}-{TARGET}"
    extracted = destination.parent / "extracted-core"
    # Share the bounded regular-header decoder with signer admission. In
    # particular, do not let tarfile allocate unbounded PAX/longname metadata.
    manifest_schema.extract_archive(archive, extracted, maximum=MAX_ARTIFACT)
    require({path.name for path in extracted.iterdir()} == {root},
            "archive release root differs from qualified version/target")
    source_path = manifest_schema.regular(extracted / root / "grip")
    require([path for path in extracted.rglob("grip")] == [source_path],
            "archive has no uniquely named core executable")
    with source_path.open("rb") as source, destination.open("xb") as output:
        shutil.copyfileobj(source, output, 1024 * 1024)
        output.flush()
        os.fchmod(output.fileno(), 0o755)
        os.fsync(output.fileno())


def admit(directory: Path, version=None, check_only=False):
    platform_check()
    release = select_release(version)
    version = release["tag_name"].removeprefix("core-v")
    name = f"gripsack-release-{version}.json"
    assets = release_assets(release)
    # Dual accepted authorities during migration: the hosted GitHub attestation
    # or the pinned owner-key local signature. Both are fully validated when
    # present; neither is substitutable by digest-only claims. A release with
    # no signature authority at all refuses before any payload transport.
    hosted_name = f"{name}.sigstore.json"
    local_name = name + LOCAL_SIGNATURE_SUFFIX
    hosted = hosted_name in assets
    local = local_name in assets
    require(name in assets and (hosted or local),
            "release has no signed qualification manifest; no historical fallback")
    if hosted:
        verifier()
    if local:
        local_verifier()
    base = f"https://github.com/{REPOSITORY}/releases/download/core-v{version}"
    path = directory / name
    download(f"{base}/{name}", path, MAX_JSON)
    manifest = manifest_schema.validate_manifest(manifest_schema.load_json(path))
    require(manifest["compatible"]["core"] == version, "manifest core version mismatch")
    require(manifest["source"] == tag_source(release["tag_name"]), "tag source identity mismatch")
    expected = {Path(item["path"]).name for item in manifest["artifacts"]} | {name}
    expected |= ({hosted_name} if hosted else set()) | ({local_name} if local else set())
    require(set(assets) == expected, "missing or extra public release assets")
    if hosted:
        bundle = directory / hosted_name
        download(f"{base}/{bundle.name}", bundle, MAX_JSON)
        verify_attestation(path, manifest, bundle)
    if local:
        signature = directory / local_name
        download(f"{base}/{signature.name}", signature, MAX_JSON)
        verify_local_signature(path, manifest, signature)
    if check_only:
        return {"version": version, "manifest_sha256": manifest_schema.digest(path)}
    for item in manifest["artifacts"]:
        payload = directory / Path(item["path"]).name
        require(item["size_bytes"] <= MAX_ARTIFACT, "artifact exceeds consumer bound")
        require(assets[payload.name].get("size") == item["size_bytes"], "public asset size mismatch")
        download(f"{base}/{payload.name}", payload, item["size_bytes"])
        require(payload.stat().st_size == item["size_bytes"]
                and manifest_schema.digest(payload) == item["sha256"], "artifact digest or size mismatch")
    core = next(item for item in manifest["artifacts"] if item["role"] == "core")
    extract_core(directory / Path(core["path"]).name, version, directory / "grip")
    # Re-observe withdrawal/tag movement before publishing, never select another version.
    fresh = select_release(version)
    require(fresh["id"] == release["id"] and tag_source(fresh["tag_name"]) == manifest["source"],
            "release identity changed during admission")
    return {"version": version, "manifest_sha256": manifest_schema.digest(path)}


class PublicationCommittedError(RuntimeError):
    """The replacement happened; failure must not imply unchanged bytes."""


def publish(source: Path, destination: Path):
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Acquire the directory before the commit point. All names below are
    # relative to this descriptor, including when another process renames it.
    descriptor = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    directory = os.fstat(descriptor)
    published = False
    pending = None

    def same_directory():
        current = destination.parent.stat()
        require((current.st_dev, current.st_ino) == (directory.st_dev, directory.st_ino),
                "installation directory changed during publication")

    try:
        try:
            lock_name = f".grip-self-update-{hashlib.sha256(os.fsencode(destination.name)).hexdigest()}.flock"
            lock_fd = os.open(lock_name,
                              os.O_RDWR | os.O_CREAT | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK,
                              0o600, dir_fd=descriptor)
            try:
                require(stat.S_ISREG(os.fstat(lock_fd).st_mode), "installation lock is not regular")
            except Exception:
                os.close(lock_fd)
                raise
            with os.fdopen(lock_fd, "a+b") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                try:
                    existing = os.stat(destination.name, dir_fd=descriptor, follow_symlinks=False)
                except FileNotFoundError:
                    pass
                else:
                    require(not stat.S_ISLNK(existing.st_mode), "installer refuses a symlink destination")
                same_directory()
                fd, temporary = tempfile.mkstemp(prefix=".grip-install-", dir=f"/proc/self/fd/{descriptor}")
                pending = Path(temporary).name
                with os.fdopen(fd, "wb") as output, source.open("rb") as input_file:
                    shutil.copyfileobj(input_file, output, 1024 * 1024)
                    output.flush()
                    os.fchmod(output.fileno(), 0o755)
                    os.fsync(output.fileno())
                same_directory()
                os.replace(pending, destination.name, src_dir_fd=descriptor, dst_dir_fd=descriptor)
                published = True
                pending = None
                os.fsync(descriptor)
                same_directory()
        finally:
            try:
                if pending is not None:
                    os.unlink(pending, dir_fd=descriptor)
            finally:
                os.close(descriptor)
    except Exception as error:
        if published:
            raise PublicationCommittedError(
                f"verified payload published; post-commit completion unconfirmed: {error}"
            ) from error
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--stage", type=Path, help="stage verified grip for the embedded updater; do not install")
    args = parser.parse_args()
    try:
        with tempfile.TemporaryDirectory(prefix="gripsack-release-") as temporary:
            directory = Path(temporary)
            receipt = admit(directory, args.version, args.check)
            if not args.check:
                if args.stage:
                    require(args.stage.is_dir(), "staging directory must already exist")
                    publish(directory / "grip", args.stage / "grip")
                else:
                    destination = Path(os.environ.get("GRIPSACK_BIN", str(Path.home() / ".local/bin"))) / "grip"
                    publish(directory / "grip", destination)
            print(json.dumps(receipt, sort_keys=True))
    except PublicationCommittedError as error:
        raise SystemExit(f"gripsack: {error}") from error
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError, subprocess.SubprocessError, RuntimeError) as error:
        raise SystemExit(f"gripsack: verified release admission failed: {error}") from error


if __name__ == "__main__":
    main()
