"""Offline transport/verifier-boundary fixtures, NOT cryptographic evidence.

The gh executable here is explicitly a test double. Actual signature acceptance
must additionally be exercised with a real signed candidate and real gh.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import tarfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPO = Path(__file__).resolve().parents[1]


def fixture_manifest(version, artifacts):
    import sys
    sys.path.insert(0, str(REPO / "scripts"))
    import release_manifest as schema
    record = lambda path, data, **extra: dict(path=path, size_bytes=len(data), sha256=hashlib.sha256(data).hexdigest(), **extra)
    evidence = [record("evidence/packages/core.json", b"{}"), record("evidence/qualification.json", b"{}")]
    return {
        "format": schema.FORMAT, "repository": schema.REPOSITORY, "policy": schema.POLICY,
        "source": {"commit": "a" * 40, "tree": "b" * 40},
        "candidate_id": "c" * 64, "qualification_sha256": "d" * 64, "recipe_sha256": "e" * 64,
        "platform": "linux-x86_64", "signer": {"workflow": schema.WORKFLOW, "ref": f"refs/tags/core-v{version}"},
        "compatible": {"core": version, "sdk": version, "bridge": version, "conda": version,
                       "ir": {"writer": 8, "readers": [3, 4, 5, 6, 7, 8]},
                       "helper_pins": {role: hashlib.sha256(data).hexdigest() for _, data, role in artifacts
                                       if role in {"bridge", "conda"}}},
        "gates": [{"id": name, "receipt_sha256": "f" * 64} for name in sorted(schema.REQUIRED_GATES)],
        "partitions": [{"id": f"Linux-{scenario}-{fault}-{shard}", "sha256": "f" * 64}
                       for scenario in ("apply-deploy", "apply-prune", "rollback-deploy", "rollback-prune", "apply-deploy-copy", "apply-prune-copy")
                       for fault in ("error", "kill") for shard in range(8)],
        "exclusions": schema.EXCLUSIONS, "provenance": {"builder": "local", "hosted_build": False},
        "artifacts": [record(f"artifacts/{name}", data, role=role) for name, data, role in artifacts],
        "packages": [record("packages/core.tgz", b"test-package", registry="npm", name="@gripsack/core", version=version,
                            dependencies=[], metadata="evidence/packages/core.json")], "evidence": evidence,
    }


def archive(version, payload=None):
    if payload is None:
        payload = f"#!/bin/sh\necho grip {version}\n".encode()
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        info = tarfile.TarInfo(f"gripsack-{version}-x86_64-unknown-linux-musl/grip")
        info.mode = 0o755
        info.size = len(payload)
        tar.addfile(info, io.BytesIO(payload))
    return buffer.getvalue()


class Releases:
    def __init__(self):
        self.releases = {}
        self.latest = ""
        self.requests = []
        self._holding = False
        self.tarball_entered = threading.Event()
        self.tarball_release = threading.Event()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                path = self.path.split("?")[0]
                owner.requests.append(path)
                prefix = "/repos/gripsack-dev/gripsack"
                if path == prefix + "/releases":
                    data = [owner.description(owner.latest)] if owner.latest else []
                elif path.startswith(prefix + "/releases/tags/core-v"):
                    version = path.removeprefix(prefix + "/releases/tags/core-v")
                    data = owner.description(version) if version in owner.releases else None
                elif path.startswith(prefix + "/releases/") and path.endswith("/assets"):
                    identity = int(path.split("/")[-2])
                    version = next(v for v, value in owner.releases.items() if value["id"] == identity)
                    data = [{"name": name, "size": len(body)} for name, body in owner.releases[version]["assets"].items()]
                elif path.startswith(prefix + "/git/ref/tags/"):
                    data = {"object": {"type": "commit", "sha": "a" * 40}}
                elif path.startswith(prefix + "/git/commits/"):
                    data = {"tree": {"sha": "b" * 40}}
                elif path.startswith("/gripsack-dev/gripsack/releases/download/core-v"):
                    tag, name = path.split("/")[-2:]
                    body = owner.releases.get(tag.removeprefix("core-v"), {}).get("assets", {}).get(name)
                    if body is None:
                        self.send_error(404)
                        return
                    if name.endswith(".tar.gz") and owner._holding:
                        owner.tarball_entered.set()
                        if not owner.tarball_release.wait(180):
                            self.send_error(500)
                            return
                    data = body
                else:
                    data = None
                if data is None:
                    self.send_error(404)
                    return
                body = data if isinstance(data, bytes) else json.dumps(data).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self._server.serve_forever, daemon=True).start()

    @property
    def base(self):
        return f"http://127.0.0.1:{self._server.server_port}"

    def description(self, version):
        return {"id": self.releases[version]["id"], "tag_name": f"core-v{version}", "draft": False, "prerelease": False}

    def serve(self, version, tarball):
        artifacts = [(f"gripsack-{version}-x86_64-unknown-linux-musl.tar.gz", tarball, "core"),
                     (f"grip-buildkit-bridge-{version}-x86_64-unknown-linux-musl", b"bridge", "bridge"),
                     (f"gripsack-conda-{version}-x86_64-unknown-linux-musl", b"conda", "conda"),
                     ("install.sh", b"reviewed fixture installer", "auxiliary")]
        manifest = fixture_manifest(version, artifacts)
        assets = {name: data for name, data, _ in artifacts}
        assets[f"gripsack-release-{version}.json"] = json.dumps(manifest).encode()
        assets[f"gripsack-release-{version}.json.sigstore.json"] = b'{"fixture":"NOT-A-SIGNATURE"}'
        self.releases[version] = {"id": len(self.releases) + 1, "assets": assets}
        self.latest = version

    def mutate_manifest(self, version, change):
        name = f"gripsack-release-{version}.json"
        manifest = json.loads(self.releases[version]["assets"][name])
        change(manifest)
        self.releases[version]["assets"][name] = json.dumps(manifest).encode()

    def hold_tarball(self):
        self.tarball_release.clear()
        self.tarball_entered.clear()
        self._holding = True

    def release_tarball(self):
        self._holding = False
        self.tarball_release.set()

    def count(self, suffix):
        return sum(path.endswith(suffix) for path in self.requests)

    def environment(self, root):
        tools = root / "release-tools"
        tools.mkdir(exist_ok=True)
        curl = shutil.which("curl")
        assert curl
        (tools / "curl").write_text(
            "#!/usr/bin/env python3\nimport os,sys\na=sys.argv[1:]\n"
            "for i,v in enumerate(a):\n"
            f" if v.startswith('https://api.github.com/'): a[i]={self.base!r}+v.removeprefix('https://api.github.com')\n"
            f" elif v.startswith('https://github.com/'): a[i]={self.base!r}+v.removeprefix('https://github.com')\n"
            " if v == '=https': a[i]='=http,https'\n"
            f"os.execv({curl!r},[{curl!r},*a])\n")
        (tools / "curl").chmod(0o755)
        (tools / "gh").write_text(
            "#!/usr/bin/env python3\n# TEST DOUBLE: never cryptographic proof.\n"
            "import hashlib,json,os,pathlib,sys\na=sys.argv[1:]\n"
            "if a==['--version']: print('gh version 2.102.0'); sys.exit(0)\n"
            "if '--help' in a: print('--source-ref --source-digest --signer-digest --cert-identity --predicate-type --deny-self-hosted-runners'); sys.exit(0)\n"
            "if os.environ.get('FIXTURE_SIGNATURE_FAILURE'): sys.exit('test-double certificate/signature refusal')\n"
            "p=pathlib.Path(a[2]); m=json.loads(p.read_text())\n"
            "expected={'--repo':'gripsack-dev/gripsack','--hostname':'github.com','--cert-identity':'https://github.com/gripsack-dev/gripsack/.github/workflows/release-core.yml@'+m['signer']['ref'],'--cert-oidc-issuer':'https://token.actions.githubusercontent.com','--source-ref':m['signer']['ref'],'--source-digest':m['source']['commit'],'--signer-digest':m['source']['commit'],'--predicate-type':'https://gripsack.dev/attestations/local-qualification/v1','--format':'json'}\n"
            "assert all(a[a.index(k)+1]==v for k,v in expected.items())\nassert '--deny-self-hosted-runners' in a\n"
            "if '--bundle' in a: assert json.loads(pathlib.Path(a[a.index('--bundle')+1]).read_text())=={'fixture':'NOT-A-SIGNATURE'}\n"
            "predicate={k:m[k] for k in ['policy','source','candidate_id','qualification_sha256','recipe_sha256']}\n"
            "predicate.update(format='gripsack.local-qualification.v1',builder='local',hosted_build=False)\n"
            "if os.environ.get('FIXTURE_WRONG_PREDICATE'): predicate['policy']='wrong-policy'\n"
            "print(json.dumps([{'verificationResult':{'statement':{'predicateType':expected['--predicate-type'],'predicate':predicate,'subject':[{'digest':{'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}}]}}}]))\n")
        (tools / "gh").chmod(0o755)
        env = dict(os.environ)
        env.update(PATH=f"{tools}{os.pathsep}{env['PATH']}", HOME=str(root), TMPDIR=str(root),
                   NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        for key in ("GH_TOKEN", "GITHUB_TOKEN", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
            env.pop(key, None)
        return env
