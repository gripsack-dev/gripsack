#!/usr/bin/env python3
"""Publish prepared bytes, not source; publication is not signed release completion.

Crates framing follows https://doc.rust-lang.org/cargo/reference/registry-web-api.html.
The npm PUT document uses the same tarball attachment protocol as libnpmpublish.
No package manager, build tool, lifecycle script, or submitted executable is run.
"""
from __future__ import annotations

import argparse
import base64
import copy
import fcntl
import hashlib
import gzip
import http.client
import os
from pathlib import Path, PurePosixPath
import struct
import sys
import tarfile
import tempfile
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request

from release_manifest import (canonical, decode_json, digest, load_json, regular,
                              safe_path, validate_bundle, validate_manifest)

MAX_PACKAGE = 256 * 1024 * 1024
MAX_EXPANDED = 512 * 1024 * 1024
MAX_METADATA = 4 * 1024 * 1024
MAX_MEMBERS = 20000
ATTEMPTS = 4
CRATES = "https://crates.io"
NPM = "https://registry.npmjs.org"
STATIC = "https://static.crates.io"
INDEX = "https://index.crates.io"


class PublicationError(ValueError):
    """A publication admission or permanent registry failure."""


class AuthenticationError(PublicationError):
    pass


class DigestMismatch(PublicationError):
    pass


class TransportError(PublicationError):
    pass


class HTTPFailure(PublicationError):
    def __init__(self, status):
        self.status = status
        super().__init__(f"registry HTTP {status}")


def _json(data):
    try:
        result = decode_json(data)
    except ValueError as error:
        raise PublicationError("invalid bounded registry/package JSON") from error
    if not isinstance(result, dict):
        raise PublicationError("metadata must be a JSON object")
    return result


def _member_path(name):
    try:
        path = PurePosixPath(safe_path(name.rstrip("/")))
    except ValueError as error:
        raise PublicationError("unsafe package archive path") from error
    if path.name in (".npmrc", "credentials", "credentials.toml"):
        raise PublicationError("credential/configuration file in package")
    return path


class _BoundedArchive:
    """Bound decompression before tarfile can allocate extended-header contents."""

    def __init__(self, stream):
        self.stream = stream
        self.remaining = MAX_EXPANDED

    def read(self, size):
        data = self.stream.read(min(size, self.remaining + 1))
        self.remaining -= len(data)
        if self.remaining < 0:
            raise PublicationError("package archive exceeds decompression bound")
        return data


def _archive_file(path, wanted):
    """Read one bounded file while admitting the entire archive without extraction."""
    try:
        path = regular(path)
    except (ValueError, OSError) as error:
        raise PublicationError("missing or linked package payload") from error
    if not 0 < path.stat().st_size <= MAX_PACKAGE:
        raise PublicationError("oversized package payload")
    found = None
    seen = set()
    total = 0
    root = PurePosixPath(wanted).parts[0]
    try:
        with path.open("rb") as packed, gzip.GzipFile(fileobj=packed) as expanded, \
                tarfile.open(fileobj=_BoundedArchive(expanded), mode="r|") as archive:
            for member in archive:
                member_path = _member_path(member.name)
                if member_path.parts[0] != root or member.name.rstrip("/") in seen:
                    raise PublicationError("duplicate or foreign package archive member")
                seen.add(member.name.rstrip("/"))
                total += member.size
                if len(seen) > MAX_MEMBERS or total > MAX_EXPANDED or member.size < 0:
                    raise PublicationError("package archive exceeds bounds")
                if not (member.isfile() or member.isdir()):
                    raise PublicationError("package archive links/special files forbidden")
                if member.name == wanted:
                    if not member.isfile() or member.size > MAX_METADATA:
                        raise PublicationError("invalid or oversized package metadata")
                    stream = archive.extractfile(member)
                    if stream is None:
                        raise PublicationError("missing package metadata stream")
                    found = stream.read(MAX_METADATA + 1)
                    if len(found) != member.size:
                        raise PublicationError("truncated package metadata")
    except (tarfile.TarError, EOFError, OSError) as error:
        raise PublicationError("invalid package archive") from error
    if found is None:
        raise PublicationError(f"package archive lacks {wanted}")
    return found


def crate_metadata(path):
    """Derive Cargo's publish request metadata from its normalized Cargo.toml."""
    path = Path(path)
    if not path.name.endswith(".crate"):
        raise PublicationError("crate payload must have .crate suffix")
    root = path.name[:-6]
    try:
        manifest = tomllib.loads(_archive_file(path, root + "/Cargo.toml").decode("utf-8"))
    except (tomllib.TOMLDecodeError, UnicodeError) as error:
        raise PublicationError("invalid normalized Cargo manifest") from error
    package = manifest.get("package", {})
    name, version = package.get("name"), package.get("version")
    if not isinstance(name, str) or not isinstance(version, str) or root != f"{name}-{version}":
        raise PublicationError("crate archive identity differs from normalized metadata")
    publish = package.get("publish")
    if publish is not None and publish != ["crates-io"]:
        raise PublicationError("crate does not permit crates.io publication")
    deps = []

    def dependencies(table, target=None):
        for section, kind in (("dependencies", "normal"), ("build-dependencies", "build"),
                              ("dev-dependencies", "dev")):
            for alias, item in table.get(section, {}).items():
                if isinstance(item, str):
                    item = {"version": item}
                if not isinstance(item, dict) or not isinstance(item.get("version"), str):
                    raise PublicationError("dependency lacks normalized registry version")
                if any(key in item for key in ("path", "git", "workspace", "registry")):
                    raise PublicationError("non-normalized or foreign registry dependency")
                registry = item.get("registry-index")
                if registry not in (None, "https://github.com/rust-lang/crates.io-index",
                                     "sparse+https://index.crates.io/"):
                    raise PublicationError("foreign dependency registry")
                deps.append({"name": item.get("package", alias), "version_req": item["version"],
                             "features": item.get("features", []), "optional": item.get("optional", False),
                             "default_features": item.get("default-features", True),
                             "target": target, "kind": kind, "registry": None,
                             "explicit_name_in_toml": alias if "package" in item else None})

    dependencies(manifest)
    for target, table in manifest.get("target", {}).items():
        dependencies(table, target)
    result = {"name": name, "vers": version, "deps": deps,
              "features": manifest.get("features", {}), "authors": package.get("authors", []),
              "keywords": package.get("keywords", []), "categories": package.get("categories", []),
              "badges": manifest.get("badges", {})}
    for key in ("description", "documentation", "homepage", "license", "repository", "links"):
        result[key] = package.get(key)
    result["license_file"] = package.get("license-file")
    result["rust_version"] = package.get("rust-version")
    readme = package.get("readme")
    if readme is False or readme is None:
        result.update(readme=None, readme_file=None)
    elif isinstance(readme, str):
        _member_path(readme)
        result["readme_file"] = readme
        result["readme"] = _archive_file(path, f"{root}/{readme}").decode("utf-8")
    else:
        raise PublicationError("README is not normalized")
    if len(canonical(result)) > MAX_METADATA:
        raise PublicationError("crate publish metadata exceeds bound")
    return result


def npm_metadata(path):
    """Return the package.json carried by the exact locally packed tarball."""
    if not Path(path).name.endswith(".tgz"):
        raise PublicationError("npm payload must have .tgz suffix")
    value = _json(_archive_file(path, "package/package.json"))
    if value.get("private") or "packageExtensions" in value:
        raise PublicationError("npm metadata prohibits public publication")
    config = value.get("publishConfig", {})
    if not isinstance(config, dict) or set(config) - {"registry", "access"}:
        raise PublicationError("unsupported npm publish configuration")
    registry = config.get("registry", NPM)
    if (not isinstance(registry, str) or registry.rstrip("/") != NPM
            or config.get("access", "public") != "public"):
        raise PublicationError("npm target must be the public npm registry")
    return value


def _identity(package):
    return package["registry"], package["name"], package["version"]


def _payload_dependencies(package, metadata):
    if package["registry"] == "crates.io":
        return {dep["name"] for dep in metadata["deps"] if dep["kind"] != "dev"}
    result = set()
    for key in ("dependencies", "optionalDependencies", "peerDependencies"):
        dependencies = metadata.get(key, {})
        if not isinstance(dependencies, dict):
            raise PublicationError("invalid npm dependency map")
        result.update(dependencies)
    return result


def prepare_packages(root, manifest):
    """Check all inputs and order before the first credential-bearing operation."""
    packages = manifest["packages"]
    names = {p["name"] for p in packages}
    if len(names) != len(packages):
        raise PublicationError("duplicate publication package name")
    prepared = {}
    seen = set()
    for package in packages:
        registry = package["registry"]
        if registry not in ("crates.io", "npm"):
            raise PublicationError("unsupported publication registry")
        path = root / package["path"]
        if path.is_symlink() or not path.is_file():
            raise PublicationError("missing publication payload")
        if path.stat().st_size != package["size_bytes"] or digest(path) != package["sha256"]:
            raise DigestMismatch("local publication payload differs from manifest")
        metadata = crate_metadata(path) if registry == "crates.io" else npm_metadata(path)
        if metadata != load_json(root / package["metadata"]):
            raise DigestMismatch("prepared metadata differs from actual package")
        if (metadata.get("name") != package["name"]
                or metadata.get("vers" if registry == "crates.io" else "version") != package["version"]):
            raise PublicationError("publication metadata identity differs from manifest")
        dependencies = set(package["dependencies"])
        if len(dependencies) != len(package["dependencies"]) or not dependencies <= seen:
            raise PublicationError("missing package or invalid publication dependency order")
        actual = _payload_dependencies(package, metadata)
        if dependencies != actual & names:
            raise PublicationError("publication dependency inventory differs from package")
        owned_missing = {name for name in actual - names
                         if name.startswith(("gripsack-", "gripfetch-", "@gripsack/"))
                         or name in ("gripsack", "griplint")}
        if owned_missing:
            raise PublicationError("release package dependency missing from bundle")
        prepared[_identity(package)] = metadata
        seen.add(package["name"])
    return prepared


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise PublicationError("registry redirects are forbidden")


class Registry:
    def __init__(self, environ=None, sleep=time.sleep):
        self.environ = os.environ if environ is None else environ
        self.sleep = sleep
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())

    def request(self, url, *, method="GET", data=None, token=None, limit=MAX_METADATA,
                content_type="application/json"):
        parsed = urllib.parse.urlsplit(url)
        if (parsed.scheme != "https"
                or parsed.netloc not in ("crates.io", "registry.npmjs.org", "static.crates.io", "index.crates.io")
                or parsed.username or parsed.password or parsed.query or parsed.fragment):
            raise PublicationError("unapproved registry URL")
        if token and (method != "PUT" or parsed.netloc not in ("crates.io", "registry.npmjs.org")):
            raise PublicationError("credentials may only accompany registry PUT")
        headers = {"User-Agent": "gripsack-release-publisher/1", "Accept": "application/json",
                   "Content-Type": content_type}
        if token:
            headers["Authorization"] = token
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        # A PUT is never blindly replayed: uncertain success is reconciled by GET.
        attempts = ATTEMPTS if method == "GET" else 1
        for attempt in range(attempts):
            try:
                with self.opener.open(request, timeout=60) as response:
                    body = response.read(limit + 1)
                    if len(body) > limit:
                        raise PublicationError("registry response exceeds byte bound")
                    return body
            except urllib.error.HTTPError as error:
                if error.code in (401, 403):
                    raise AuthenticationError("registry authentication rejected") from None
                if error.code == 404 and method == "GET":
                    return None
                if error.code not in (408, 429, 500, 502, 503, 504):
                    raise HTTPFailure(error.code) from None
                failure = TransportError(f"transient registry HTTP {error.code}")
            except (urllib.error.URLError, TimeoutError, ConnectionError, http.client.HTTPException):
                failure = TransportError("registry transport failed")
            if attempt + 1 == attempts:
                raise failure
            self.sleep(2 ** attempt)
        raise AssertionError("unreachable request loop")

    def crate_index(self, package):
        name = package["name"].lower()
        if len(name) < 3:
            prefix = str(len(name))
        elif len(name) == 3:
            prefix = "3/" + name[0]
        else:
            prefix = name[:2] + "/" + name[2:4]
        for attempt in range(ATTEMPTS):
            if attempt:
                self.sleep(2 ** (attempt - 1))
            data = self.request(f"{INDEX}/{prefix}/{name}")
            if data is None:
                continue
            found = []
            for line in data.splitlines():
                entry = _json(line)
                if entry.get("vers") == package["version"]:
                    found.append(entry)
            if not found:
                continue
            if (len(found) != 1 or found[0].get("name") != package["name"]
                    or found[0].get("yanked") is not False):
                raise PublicationError("crate index identity/yank state differs")
            if found[0].get("cksum") != package["sha256"]:
                raise DigestMismatch("crate index checksum differs from prepared payload")
            return
        raise TransportError("published crate is not yet visible in the crates.io index")

    def remote_digest(self, package):
        name, version = package["name"], package["version"]
        escaped = urllib.parse.quote(name, safe="")
        if package["registry"] == "crates.io":
            data = self.request(f"{CRATES}/api/v1/crates/{escaped}/{version}")
            if data is None:
                return None
            info = _json(data).get("version", {})
            if info.get("num") != version or info.get("crate") != name or info.get("yanked") is not False:
                raise PublicationError("registry crate identity/yank state differs")
            if info.get("checksum") != package["sha256"]:
                raise DigestMismatch("registry crate checksum differs from prepared payload")
            self.crate_index(package)
            url = f"{STATIC}/crates/{name}/{name}-{version}.crate"
            advertised = None
        else:
            data = self.request(f"{NPM}/{escaped}/{version}")
            if data is None:
                return None
            info = _json(data)
            if info.get("name") != name or info.get("version") != version:
                raise PublicationError("registry npm identity differs")
            advertised = info.get("dist", {})
            url = f"{NPM}/{name}/-/{name.rsplit('/', 1)[-1]}-{version}.tgz"
            if advertised.get("tarball") != url:
                raise PublicationError("npm tarball is not at the fixed registry target")
        payload = self.request(url, limit=package["size_bytes"])
        if payload is None:
            raise TransportError("registry metadata exists but published bytes are not observable")
        actual = hashlib.sha256(payload).hexdigest()
        if len(payload) != package["size_bytes"] or actual != package["sha256"]:
            raise DigestMismatch("downloaded registry bytes differ from prepared payload")
        if advertised is not None:
            integrity = "sha512-" + base64.b64encode(hashlib.sha512(payload).digest()).decode("ascii")
            if advertised.get("integrity") != integrity:
                raise DigestMismatch("npm integrity differs from downloaded bytes")
            if advertised.get("shasum") != hashlib.sha1(payload).hexdigest():
                raise DigestMismatch("npm shasum differs from downloaded bytes")
        return actual

    def upload(self, package, path, metadata):
        with regular(path).open("rb") as stream:
            payload = stream.read(min(package["size_bytes"], MAX_PACKAGE) + 1)
        if len(payload) != package["size_bytes"] or hashlib.sha256(payload).hexdigest() != package["sha256"]:
            raise DigestMismatch("publication payload changed after admission")
        if package["registry"] == "crates.io":
            token = self.environ.get("CARGO_REGISTRY_TOKEN")
            if not token:
                raise AuthenticationError("CARGO_REGISTRY_TOKEN is required for crate upload")
            encoded = canonical(metadata)
            body = struct.pack("<I", len(encoded)) + encoded + struct.pack("<I", len(payload)) + payload
            response = self.request(f"{CRATES}/api/v1/crates/new", method="PUT", data=body,
                                    token=token, content_type="application/octet-stream")
            if _json(response).get("errors"):
                raise PublicationError("crates.io rejected publication metadata")
        else:
            token = self.environ.get("NPM_TOKEN")
            if not token:
                raise AuthenticationError("NPM_TOKEN is required for npm upload")
            name, version = package["name"], package["version"]
            filename = f"{name.rsplit('/', 1)[-1]}-{version}.tgz"
            item = copy.deepcopy(metadata)
            item["_id"] = f"{name}@{version}"
            item["dist"] = {"tarball": f"{NPM}/{name}/-/{filename}",
                            "shasum": hashlib.sha1(payload).hexdigest(),
                            "integrity": "sha512-" + base64.b64encode(hashlib.sha512(payload).digest()).decode("ascii")}
            body = {"_id": name, "name": name, "description": metadata.get("description", ""),
                    "access": "public", "dist-tags": {"latest": version}, "versions": {version: item},
                    "_attachments": {filename: {"content_type": "application/octet-stream",
                                               "data": base64.b64encode(payload).decode("ascii"),
                                               "length": len(payload)}}}
            response = self.request(f"{NPM}/{urllib.parse.quote(name, safe='')}", method="PUT",
                                    data=canonical(body), token="Bearer " + token)
            if _json(response).get("error"):
                raise PublicationError("npm rejected publication metadata")

    def publish(self, package, path, metadata):
        existing = self.remote_digest(package)
        if existing is not None:
            return existing
        upload_failure = None
        try:
            self.upload(package, path, metadata)
        except TransportError as error:
            upload_failure = error
        except HTTPFailure as error:
            if error.status not in (400, 409):
                raise
            # An upload race is only success if actual registry bytes now match.
            upload_failure = error
        for attempt in range(ATTEMPTS):
            if attempt:
                self.sleep(2 ** (attempt - 1))
            actual = self.remote_digest(package)
            if actual is not None:
                return actual
        if upload_failure is not None:
            raise upload_failure
        raise TransportError("uploaded package not yet observable; resume this result, do not rebuild")


def _save_result(path, result):
    fd, temporary = tempfile.mkstemp(prefix=".publication-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(canonical(result) + b"\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def _record(package, remote_sha256):
    return {"registry": package["registry"], "name": package["name"], "version": package["version"],
            "size_bytes": package["size_bytes"], "remote_sha256": remote_sha256}


def publish_bundle(root, result_path, registries=None, registry=None):
    root = Path(root).resolve()
    manifest = validate_manifest(validate_bundle(root))
    metadata = prepare_packages(root, manifest)
    selected = set(registries or ("crates.io", "npm"))
    if not selected <= {"crates.io", "npm"}:
        raise PublicationError("unsupported registry selection")
    manifest_path = root / f"gripsack-release-{manifest['compatible']['core']}.json"
    binding = {"manifest_sha256": digest(manifest_path), "source": manifest["source"],
               "candidate_id": manifest["candidate_id"]}
    result_path = Path(result_path).absolute()
    if result_path.is_symlink() or root == result_path.resolve() or root in result_path.resolve().parents:
        raise PublicationError("publication result must be outside immutable bundle and not a symlink")
    result_path.parent.mkdir(parents=True, exist_ok=True)
    registry = Registry() if registry is None else registry
    lock_path = result_path.with_name(result_path.name + ".lock")
    lock_fd = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(lock_fd, "w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise PublicationError("publication result is already in use") from error
        result = {"format": "gripsack.publication-result.v1", "identity": binding, "status": "partial",
                  "signed_release_complete": False, "packages": []}
        if result_path.exists():
            previous = load_json(result_path)
            if (set(previous) != set(result) or previous["format"] != result["format"]
                    or previous["identity"] != binding or previous["signed_release_complete"] is not False
                    or previous["status"] not in ("partial", "publication_complete")
                    or not isinstance(previous["packages"], list)):
                raise PublicationError("publication resume identity/schema mismatch")
            allowed = [_record(p, p["sha256"]) for p in manifest["packages"]]
            indices = []
            for record in previous["packages"]:
                if record not in allowed:
                    raise PublicationError("publication resume package/digest mismatch")
                indices.append(allowed.index(record))
            if indices != sorted(set(indices)):
                raise PublicationError("publication resume duplicate/unordered package")
            result["packages"] = previous["packages"]
        _save_result(result_path, result)
        completed = {_identity(record): record for record in result["packages"]}
        for package in manifest["packages"]:
            key = _identity(package)
            if package["registry"] not in selected and key not in completed:
                continue
            was_verified = key in completed
            completed.pop(key, None)
            result["packages"] = [completed[_identity(p)] for p in manifest["packages"] if _identity(p) in completed]
            _save_result(result_path, result)
            if was_verified:
                actual = registry.remote_digest(package)
                if actual is None:
                    raise PublicationError("previously verified registry package is missing")
            else:
                actual = registry.publish(package, root / package["path"], metadata[key])
            if actual != package["sha256"]:
                raise DigestMismatch("remote publication digest differs")
            completed[key] = _record(package, actual)
            result["packages"] = [completed[_identity(p)] for p in manifest["packages"] if _identity(p) in completed]
            _save_result(result_path, result)
        if len(completed) == len(manifest["packages"]):
            result["status"] = "publication_complete"
        _save_result(result_path, result)
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--result", type=Path, required=True,
                        help="durable identity-bound result outside the immutable bundle")
    parser.add_argument("--registry", choices=("crates.io", "npm"), action="append")
    args = parser.parse_args()
    try:
        result = publish_bundle(args.bundle, args.result, args.registry)
    except (ValueError, OSError, KeyError, TypeError) as error:
        # Do not echo untrusted remote responses or exception strings containing secrets.
        print(f"publication failed ({type(error).__name__}); preserve result for exact-bundle resume", file=sys.stderr)
        return 1
    print(canonical(result).decode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
