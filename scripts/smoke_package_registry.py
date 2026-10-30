#!/usr/bin/env python3
"""Real private-TLS OCI package journey; caller provisions registry and signed inputs.

No provisioning, publication to public registries, trust synthesis, or pytest.
Preserves supplied inputs and failed evidence. See verification/release-handoff.md.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import ssl
import stat
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request

OCI = "application/vnd.oci.image.manifest.v1+json"
PORTABLE = "application/vnd.gripsack.portable.v1+json"
DSSE = "application/vnd.dsse.envelope.v1+json"
BLOB = "application/vnd.gripsack.portable.blob.v1"
EMPTY = "application/vnd.oci.empty.v1+json"
MAX_BYTES = 512 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def private_file(path, data):
    with path.open("xb") as stream:
        os.fchmod(stream.fileno(), 0o600)
        stream.write(data)


def remove_owned_tree(path):
    # Imported store directories are immutable. Never follow payload symlinks.
    for directory, children, _ in os.walk(path, followlinks=False):
        os.chmod(directory, 0o700)
        for name in children:
            child = Path(directory) / name
            if not child.is_symlink():
                child.chmod(0o700)
    shutil.rmtree(path)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise RuntimeError("registry redirect refused; credentials were not forwarded")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("grip", "bundle", "trust", "credentials", "ca", "workdir"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--registry", required=True)
    args = parser.parse_args()
    os.umask(0o077)
    binary = args.grip.resolve(strict=True)
    bundle = args.bundle.resolve(strict=True)
    trust = args.trust.resolve(strict=True)
    credentials = args.credentials.resolve(strict=True)
    ca = args.ca.resolve(strict=True)
    work = args.workdir.absolute()
    parsed = urllib.parse.urlsplit(args.registry)
    require(parsed.scheme == "https" and parsed.hostname and not parsed.username
            and not parsed.password and parsed.path in ("", "/")
            and not parsed.query and not parsed.fragment,
            "--registry must be a private HTTPS origin without credentials or path")
    origin = args.registry.rstrip("/")
    require(binary.is_file() and os.access(binary, os.X_OK), "--grip must be executable")
    require(bundle.is_dir(), "--bundle must be a signed native bundle directory")
    require(not work.exists() and not work.is_symlink(), "--workdir must be fresh")
    # An exclusive mkdir owns exactly this tree; caller inputs cannot be its children.
    work.mkdir(mode=0o700)
    work = work.resolve(strict=True)
    with credentials.open("rb") as stream:
        auth_bytes = stream.read(16385)
    require(len(auth_bytes) <= 16384, "credentials exceed CLI input bound")
    auth = json.loads(auth_bytes)
    require(isinstance(auth, dict) and set(auth) == {"username", "password"}
            and all(isinstance(v, str) and v for v in auth.values())
            and ":" not in auth["username"], "requires nonempty basic credentials JSON")
    wrong = {"username": "refused-" + secrets.token_hex(16),
             "password": secrets.token_hex(32)}
    sensitive = []
    for item in (auth, wrong):
        sensitive.extend(item.values())
        sensitive.append(base64.b64encode(
            (item["username"] + ":" + item["password"]).encode()).decode())
        sensitive.extend(json.dumps(v)[1:-1] for v in item.values())
    sensitive = sorted(set(sensitive), key=len, reverse=True)

    def clean_output(text):
        require(not any(value in text for value in sensitive),
                "credential value detected in captured output; unsafe output withheld")
        require("-----BEGIN PRIVATE KEY-----" not in text
                and "-----BEGIN OPENSSH PRIVATE KEY-----" not in text,
                "private key detected in captured output; unsafe output withheld")
        return text

    log_path = work / "commands.jsonl"
    private_file(log_path, b"")

    def command(env, argv, *, refused=False, as_json=True):
        result = subprocess.run(list(map(str, argv)), cwd=env["HOME"], env=env,
                                capture_output=True, timeout=360)
        stdout = clean_output(result.stdout.decode("utf-8", errors="replace"))
        stderr = clean_output(result.stderr.decode("utf-8", errors="replace"))
        entry = {"argv": list(map(str, argv)), "home": env["HOME"],
                 "returncode": result.returncode, "stdout": stdout, "stderr": stderr}
        # Raw output is retained only after secret checks, never printed on failure.
        with log_path.open("a") as stream:
            stream.write(clean_output(json.dumps(entry)) + "\n")
        if refused:
            require(result.returncode == 1,
                    "expected registry credential refusal; see private command log")
            return {"returncode": result.returncode, "authentication_refused": True}
        require(result.returncode == 0, "CLI command failed; see private command log")
        return json.loads(stdout) if as_json else stdout.strip()

    trust_bytes = trust.read_bytes()
    require(isinstance(json.loads(trust_bytes), dict), "trust policy must be a JSON object")

    def home(name):
        root = work / name
        root.mkdir(mode=0o700)
        # Do not inherit caches, proxies, credentials, loader injection, or authority.
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8",
               "HOME": str(root), "GRIPSACK_HOME": str(root / "state"),
               "SSL_CERT_FILE": str(ca)}
        for variable, child in (("XDG_CACHE_HOME", "cache"), ("XDG_CONFIG_HOME", "config"),
                                ("XDG_DATA_HOME", "data"), ("TMPDIR", "tmp")):
            (root / child).mkdir(mode=0o700)
            env[variable] = str(root / child)
        policy = root / "state" / "sharing" / "trust.json"
        policy.parent.mkdir(parents=True, mode=0o700)
        private_file(policy, trust_bytes)
        private_file(root / "credentials.json", auth_bytes)
        private_file(root / "wrong-credentials.json", json.dumps(wrong).encode())
        return env

    sender, receiver = home("sender"), home("receiver")
    transfer = work / "transfer"
    # Reject symlink inputs rather than copying any unexpected caller-owned target.
    require(not any(p.is_symlink() for p in bundle.rglob("*")),
            "bundle transport directory must not contain symlinks")
    shutil.copytree(bundle, transfer)
    manifest_bytes = (transfer / "manifest.json").read_bytes()
    manifest = json.loads(manifest_bytes)
    identity = digest(manifest_bytes)
    require("grip" in manifest["commands"] and manifest["layout"]["kind"] == "relocatable",
            "bundle must expose native relocatable command grip")
    require(all(c.get("conda") is None and c.get("external") is None
                for c in manifest["components"]),
            "native grip journey requires local native components, not environment runtimes")
    selected = manifest["commands"]["grip"]
    repository = "gripsack-private-smoke/" + secrets.token_hex(16)
    transport_args = ["--registry", origin, "--repository", repository]

    def package(env, *arguments, refused=False):
        return command(env, [binary, "package", *arguments], refused=refused)

    imported = package(sender, "import", "--bundle", transfer)
    require(imported["manifest"] == identity, "import changed package identity")
    source_facts = package(sender, "inspect", "--manifest", identity)
    require(source_facts["manifest"] == identity, "source inspection identity mismatch")
    pushed = package(sender, "push", "--manifest", identity, "--reference", "journey",
                     *transport_args, "--basic-credentials-file",
                     Path(sender["HOME"]) / "credentials.json")
    transport_digest = pushed["transport_digest"]
    require(pushed["manifest"] == identity
            and re.fullmatch(r"sha256:[0-9a-f]{64}", transport_digest),
            "invalid push transport identity")

    # Independent HTTP reader: real CA validation, no proxies, no redirects.
    opener = urllib.request.build_opener(
        urllib.request.ProxyHandler({}), NoRedirect(),
        urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=str(ca))))
    basic = "Basic " + base64.b64encode(
        (auth["username"] + ":" + auth["password"]).encode()).decode()

    def get(suffix, bound, accept, authorization=basic):
        headers = {"Accept": accept}
        if authorization is not None:
            headers["Authorization"] = authorization
        request = urllib.request.Request(origin + "/v2/" + repository + "/" + suffix,
                                         headers=headers)
        with opener.open(request, timeout=120) as response:
            require(response.status == 200, "independent registry GET failed")
            data = response.read(bound + 1)
            require(len(data) <= bound, "independent registry response exceeds bound")
            return data, response.headers

    refusals = {}
    for label, credential in (("missing", None), ("wrong", wrong)):
        token = None if credential is None else "Basic " + base64.b64encode(
            (credential["username"] + ":" + credential["password"]).encode()).decode()
        try:
            get("manifests/" + transport_digest, 1024 * 1024, OCI, token)
        except urllib.error.HTTPError as error:
            require(error.code == 401, "independent auth refusal must be HTTP 401")
            refusals[label] = {"http_status": error.code}
        else:
            raise RuntimeError("private registry accepted missing/wrong credentials")

    raw, headers = get("manifests/" + transport_digest, 1024 * 1024, OCI)
    require("sha256:" + digest(raw) == transport_digest
            and headers.get_content_type() == OCI, "OCI manifest digest/type mismatch")
    transport = json.loads(raw)
    require(transport["schemaVersion"] == 2 and transport["mediaType"] == OCI
            and transport["artifactType"] == "application/vnd.gripsack.portable.v1",
            "unsupported OCI manifest structure")
    expected = [(EMPTY, b"{}"), (PORTABLE, manifest_bytes),
                (DSSE, (transfer / "envelope.json").read_bytes())]
    envelope = json.loads(expected[2][1])
    require(envelope["payloadType"] == PORTABLE
            and base64.b64decode(envelope["payload"], validate=True) == manifest_bytes
            and bool(envelope["signatures"]), "DSSE exact payload binding mismatch")
    # This native lane has no external closure. Compare actual bundle blobs, not
    # bytes reconstructed with a production serializer or verifier.
    for path in sorted((transfer / "blobs").iterdir()):
        require(path.is_file() and file_digest(path) == path.name,
                "source payload blob filename/digest mismatch")
        expected.append((BLOB, path))
    descriptors = [transport["config"], *transport["layers"]]
    require(len(descriptors) == len(expected) <= 4097, "OCI descriptor inventory mismatch")
    total = len(raw)
    verified = []
    for descriptor, (kind, source) in zip(descriptors, expected):
        size = source.stat().st_size if isinstance(source, Path) else len(source)
        sha = file_digest(source) if isinstance(source, Path) else digest(source)
        require(descriptor == {"mediaType": kind, "digest": "sha256:" + sha, "size": size},
                "OCI descriptor digest/size/type differs from exact source object")
        total += size
        require(total <= MAX_BYTES, "OCI transfer exceeds native transport budget")
        data, _ = get("blobs/sha256:" + sha, size, "application/octet-stream")
        require(len(data) == size and digest(data) == sha,
                "independent blob digest/size mismatch")
        if isinstance(source, bytes):
            require(data == source, "embedded metadata differs from exact supplied bytes")
        verified.append({"media_type": kind, "digest": "sha256:" + sha, "size": size})

    remove_owned_tree(Path(sender["HOME"]))
    remove_owned_tree(transfer)
    require(not Path(sender["HOME"]).exists() and not transfer.exists(),
            "sender/cache/transfer removal failed")
    for label, filename in (("missing", None), ("wrong", "wrong-credentials.json")):
        extra = [] if filename is None else ["--basic-credentials-file",
                                            Path(receiver["HOME"]) / filename]
        refusals[label]["cli"] = package(receiver, "pull", "--digest", transport_digest,
                                         *transport_args, *extra, refused=True)
    pulled = package(receiver, "pull", "--digest", transport_digest, *transport_args,
                     "--basic-credentials-file", Path(receiver["HOME"]) / "credentials.json")
    facts = package(receiver, "inspect", "--manifest", identity)
    require(pulled["manifest"] == facts["manifest"] == identity,
            "recipient package identity mismatch")
    payload = Path(facts["payload"]).resolve(strict=True)
    require(payload.is_relative_to(Path(receiver["GRIPSACK_HOME"])),
            "recipient payload escaped its private store")
    executable = payload / "components" / selected["component"] / selected["executable"]
    require(executable.resolve(strict=True).is_relative_to(payload), "command escaped payload")
    # Store policy seals files; directories remain writable for repair/GC.
    for path in payload.rglob("*"):
        if path.is_file() and not path.is_symlink():
            require(not stat.S_IMODE(path.stat().st_mode) & 0o222,
                    "recipient payload file is not read-only")

    def payload_inventory():
        result = {}
        for path in [payload, *payload.rglob("*")]:
            mode = stat.S_IMODE(path.lstat().st_mode)
            if path.is_symlink():
                value = ("symlink", mode, os.readlink(path))
            elif path.is_file():
                value = ("file", mode, file_digest(path))
            else:
                require(path.is_dir(), "unexpected payload filesystem object")
                value = ("directory", mode)
            result[str(path.relative_to(payload))] = value
        return result

    before_execution = payload_inventory()
    executable_digest = file_digest(executable)
    version = command(receiver, [executable, *selected["args"], "--version"], as_json=False)
    require(re.fullmatch(r"grip(?:sack)? \S+", version) is not None,
            "imported native grip did not emit a version")
    require(payload_inventory() == before_execution, "native execution changed its retained payload")
    require(not (Path(receiver["GRIPSACK_HOME"]) / "current").exists(),
            "package journey unexpectedly activated a profile")
    evidence = {"status": "passed", "core_sha256": file_digest(binary),
                "registry_origin": origin, "repository": repository,
                "transport_digest": transport_digest, "package_manifest": identity,
                "publisher": facts["publisher"], "namespace": facts["namespace"],
                "trust_sha256": digest(trust_bytes), "ca_sha256": file_digest(ca),
                "authentication_refusals": refusals,
                "sender_and_transfer_removed_before_pull": True,
                "independent_verification": {"method": "urllib+ssl+hashlib",
                    "tls_ca_verified": True, "manifest_sha256": digest(raw),
                    "exact_portable_and_dsse_bytes": True,
                    "signature_authority": "ordinary grip import/inspect with supplied policy",
                    "descriptors": verified, "total_bytes": total},
                "native_execution": {"version": version, "sha256": executable_digest,
                                     "read_only_payload_files": True, "payload_unchanged": True},
                "credentials_absent_from_captured_output": True,
                "private_command_log": str(log_path)}
    encoded = clean_output(json.dumps(evidence, sort_keys=True))
    require(len(encoded.encode()) <= 1024 * 1024, "evidence summary exceeds bound")
    private_file(work / "evidence.json", (encoded + "\n").encode())
    print(encoded)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # Do not echo exception payloads: parser/network/JSON failures may contain
        # caller-controlled credentials. Detailed safe CLI output stays private.
        message = str(error) if isinstance(error, RuntimeError) else type(error).__name__
        print(json.dumps({"status": "failed", "error": message}), file=sys.stderr)
        sys.exit(1)
