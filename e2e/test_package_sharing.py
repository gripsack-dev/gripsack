"""Real offline Linux sharing journeys; compiler/OpenSSL prerequisites must fail loudly.

The relative launcher is deliberately qualified, not a claim that copying arbitrary
prefixes works. Producer source, keys, HOME and cache are removed before invocation.
Registry qualification is a separate explicitly selected test requiring a real local
Distribution server; an absent server is never reported as a passing registry case.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import subprocess
import tarfile
from pathlib import Path

import pytest

from conftest import GRIP, make_workspace_repo, run_grip

pytestmark = pytest.mark.skipif(
    platform.system() != "Linux" or platform.machine() != "x86_64",
    reason="qualified native Linux x86_64 sharing contract",
)

CONTRACT = (
    "linux-x86_64-v1: static launcher resolves /proc/self/exe, including symlink "
    "entry and moved/spaced roots; exec preserves arguments, exit and signals. "
    "Dynamic programs use bundled GNU loader with --inhibit-cache and explicit "
    "relative library directory. Dynamic argv[0] is the resolved program path; "
    "dynamic /proc/self/exe denotes the loader and self-reexec is unsupported. "
    "LD_PRELOAD, LD_AUDIT, LD_LIBRARY_PATH and GLIBC_TUNABLES are cleared. "
    "Bundled shell interpreter launches a relative static child; no host shell."
)

PROBE = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
int main(int argc, char **argv) {
    if (argc > 1 && !strcmp(argv[1], "exit")) return 37;
    if (argc > 1 && !strcmp(argv[1], "signal")) { raise(SIGTERM); return 99; }
    for (int i = 1; i < argc; ++i) printf("%zu:%s\n", strlen(argv[i]), argv[i]);
    return 0;
}
'''

LAUNCHER = r'''
#define _GNU_SOURCE
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
    char root[PATH_MAX], loader[PATH_MAX], libs[PATH_MAX], program[PATH_MAX];
    ssize_t n = readlink("/proc/self/exe", root, sizeof(root) - 1);
    if (n < 0 || n == sizeof(root) - 1 || argc < 2) return 90;
    root[n] = 0;
    char *slash = strrchr(root, '/'); if (!slash) return 91; *slash = 0;
    slash = strrchr(root, '/'); if (!slash) return 91; *slash = 0;
    snprintf(loader, sizeof(loader), "%s/lib/ld-linux-x86-64.so.2", root);
    snprintf(libs, sizeof(libs), "%s/lib", root);
    int shell = !strcmp(argv[1], "interpreter");
    snprintf(program, sizeof(program), "%s/bin/%s", root, shell ? "sh" : "dynamic");
    unsetenv("LD_PRELOAD"); unsetenv("LD_AUDIT"); unsetenv("LD_LIBRARY_PATH");
    unsetenv("GLIBC_TUNABLES");
    if (setenv("BUNDLE_ROOT", root, 1)) return 92;
    char script[PATH_MAX]; snprintf(script, sizeof(script), "%s/data/child.sh", root);
    char **args = calloc((size_t)argc + 8, sizeof(char *)); if (!args) return 93;
    int p = 0; args[p++] = loader; args[p++] = "--inhibit-cache";
    args[p++] = "--library-path"; args[p++] = libs; args[p++] = program;
    if (shell) args[p++] = script;
    for (int i = 2; i < argc; ++i) args[p++] = argv[i];
    execv(loader, args); perror("exec bundled loader"); return 94;
}
'''

DYNAMIC = r'''
#include <dlfcn.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
extern int transitive(void);
int main(void) {
    char path[PATH_MAX]; const char *root = getenv("BUNDLE_ROOT");
    if (!root || transitive() != 42) return 81;
    snprintf(path, sizeof(path), "%s/lib/plugin.so", root);
    void *plugin = dlopen(path, RTLD_NOW); if (!plugin) { puts(dlerror()); return 82; }
    int (*value)(void) = (int (*)(void))dlsym(plugin, "plugin_value");
    if (!value || value() != 73) return 83;
    snprintf(path, sizeof(path), "%s/data/message.txt", root);
    FILE *data = fopen(path, "r"); if (!data) return 84;
    char line[80]; if (!fgets(line, sizeof(line), data)) return 85;
    fclose(data); dlclose(plugin); printf("transitive=42 plugin=73 %s", line); return 0;
}
'''


def checked(command, **kwargs):
    result = subprocess.run(list(map(str, command)), capture_output=True, timeout=120, **kwargs)
    assert result.returncode == 0, (command, result.stdout, result.stderr)
    return result


def recipient(root):
    root.mkdir(parents=True)
    env = dict(os.environ, HOME=str(root), GRIPSACK_HOME=str(root / "state"))
    for name in ("XDG_DATA_HOME", "GRIPSACK_TRUST_ALL", "LD_PRELOAD", "LD_AUDIT", "LD_LIBRARY_PATH"):
        env.pop(name, None)
    return env


def package(env, *args, ok=True):
    result = run_grip([str(GRIP.resolve()), "package", *map(str, args)], env=env,
                      cwd=env["HOME"], capture_output=True, text=True, timeout=120)
    if ok:
        assert result.returncode == 0, result.stdout + result.stderr
    else:
        assert result.returncode != 0, result.stdout + result.stderr
    return result


def compile_payload(producer):
    source = producer / "source"
    component = source / "components" / "native"
    for name in ("bin", "lib", "data"):
        (component / name).mkdir(parents=True)
    cc = shutil.which("cc")
    assert cc, "native sharing qualification requires cc and static libc development files"

    def compile_one(name, text, flags, output):
        path = producer / (name + ".c")
        path.write_text(text)
        checked([cc, str(path), *flags, "-o", str(component / output)])

    compile_one("probe", PROBE, ["-static"], "bin/static")
    compile_one("launcher", LAUNCHER, ["-static"], "bin/launch")
    compile_one("leaf", "int leaf(void) { return 42; }\n",
                ["-shared", "-fPIC", "-Wl,-soname,libleaf.so"], "lib/libleaf.so")
    compile_one("middle", "extern int leaf(void); int transitive(void) { return leaf(); }\n",
                ["-shared", "-fPIC", "-L" + str(component / "lib"), "-lleaf",
                 "-Wl,-soname,libmiddle.so"], "lib/libmiddle.so")
    compile_one("plugin", "int plugin_value(void) { return 73; }\n",
                ["-shared", "-fPIC"], "lib/plugin.so")
    compile_one("dynamic", DYNAMIC,
                ["-L" + str(component / "lib"), "-lmiddle", "-ldl",
                 "-Wl,-rpath-link," + str(component / "lib")], "bin/dynamic")
    shutil.copyfile(Path("/bin/sh").resolve(), component / "bin/sh")
    (component / "bin/sh").chmod(0o755)
    # Only our compiled image and the trusted host interpreter are passed to ldd.
    for binary in (component / "bin/dynamic", component / "bin/sh"):
        inventory = checked(["ldd", binary], text=True,
                            env=dict(os.environ, LD_LIBRARY_PATH=str(component / "lib"))).stdout
        for name in re.findall(r"(?:=>\s+)?(/[^\s(]+)", inventory):
            library = Path(name)
            target = component / "lib" / library.name
            if library.resolve() != target.resolve():
                shutil.copyfile(library, target)
                target.chmod(0o755)
    assert (component / "lib/ld-linux-x86-64.so.2").exists(), "GNU bundled-loader lane required"
    (component / "data/message.txt").write_text("resource=portable\n")
    (component / "data/child.sh").write_text('exec "$BUNDLE_ROOT/bin/static" "$@"\n')
    return source


def native_target():
    return {"os": "linux", "arch": "x86_64", "abi": {"kind": "linux"},
            "minimum_kernel": "3.2", "cpu": "x86_64"}


def specification():
    return {
        "version": 1, "namespace": "fixture/native", "publisher": "fixture-publisher",
        "target": native_target(),
        "layout": {"kind": "relocatable", "contract": {"behavior": CONTRACT}},
        "components": [{"id": "native", "payload": None, "tree": None,
                        "runtime": [], "conda": None, "external": None, "inventory": []}],
        "roots": ["native"],
        "commands": {"dynamic": {"component": "native", "executable": "bin/launch", "args": ["dynamic"]},
                     "interpreter": {"component": "native", "executable": "bin/launch", "args": ["interpreter"]},
                     "static": {"component": "native", "executable": "bin/static", "args": []}},
        "environment": {"SHARING_PASSIVE": {"kind": "literal", "value": "not-installed-by-import"}},
        "licenses": [], "redistribution_authorized": True,
    }


def canonical(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def pae(payload_type, payload):
    kind = payload_type.encode()
    return b"DSSEv1 " + str(len(kind)).encode() + b" " + kind + b" " + str(len(payload)).encode() + b" " + payload


def trust(env, public_key, *, revoked=False):
    directory = Path(env["GRIPSACK_HOME"]) / "sharing"
    directory.mkdir(parents=True, exist_ok=True)
    policy = {
        "version": 1,
        "keys": [{"key_id": "fixture", "publisher": "fixture-publisher",
                  "namespaces": ["fixture/native"], "public_key": public_key,
                  "allowed_layouts": ["relocatable"],
                  "required_validators": ["manifest-integrity", "payload-integrity", "layout-safety"]}],
        "revoked_keys": ["fixture"] if revoked else [], "revoked_manifests": [],
    }
    (directory / "trust.json").write_bytes(canonical(policy))


def independent_sign(bundle, private_key, scratch):
    payload = (bundle / "manifest.json").read_bytes()
    payload_type = "application/vnd.gripsack.portable.v1+json"
    message = scratch / "pae"
    message.write_bytes(pae(payload_type, payload))
    signature = scratch / "signature"
    checked(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", private_key,
             "-in", message, "-out", signature])
    (bundle / "envelope.json").write_bytes(canonical({
        "payloadType": payload_type, "payload": base64.b64encode(payload).decode(),
        "signatures": [{"keyid": "fixture", "sig": base64.b64encode(signature.read_bytes()).decode()}],
    }))


@pytest.fixture
def published(sandbox):
    producer = sandbox / "producer"
    env = recipient(producer)
    source = compile_payload(producer)
    private_key = producer / "private.pem"
    checked(["openssl", "genpkey", "-algorithm", "ED25519", "-out", private_key])
    private_der = producer / "private.der"
    public_der = producer / "public.der"
    public_pem = producer / "public.pem"
    checked(["openssl", "pkey", "-in", private_key, "-outform", "DER", "-out", private_der])
    checked(["openssl", "pkey", "-in", private_key, "-pubout", "-outform", "DER", "-out", public_der])
    checked(["openssl", "pkey", "-in", private_key, "-pubout", "-out", public_pem])
    der = private_der.read_bytes()
    assert der[:16] == bytes.fromhex("302e020100300506032b657004220420") and len(der) == 48
    raw_key = producer / "seed"
    raw_key.write_bytes(der[-32:])
    raw_key.chmod(0o600)
    public = public_der.read_bytes()
    assert public[:12] == bytes.fromhex("302a300506032b6570032100") and len(public) == 44
    public_key = base64.b64encode(public[-32:]).decode()
    spec = producer / "specification.json"
    spec.write_bytes(canonical(specification()))
    bundle = sandbox / "published-bundle"
    package(env, "pack", "--manifest", spec, "--source", source, "--output", bundle,
            "--key", raw_key, "--key-id", "fixture")
    # Independently verify the Rust signer's standard DSSE PAE with OpenSSL.
    envelope = json.loads((bundle / "envelope.json").read_bytes())
    payload = (bundle / "manifest.json").read_bytes()
    assert base64.b64decode(envelope["payload"], validate=True) == payload
    message = producer / "pae"
    signature = producer / "signature"
    message.write_bytes(pae(envelope["payloadType"], payload))
    signature.write_bytes(base64.b64decode(envelope["signatures"][0]["sig"], validate=True))
    checked(["openssl", "pkeyutl", "-verify", "-rawin", "-pubin", "-inkey", public_pem,
             "-in", message, "-sigfile", signature])
    # Recipient imports a signature generated outside Gripsack, not its own roundtrip.
    independent_sign(bundle, private_key, producer)
    manifest = json.loads(payload)
    for component in manifest["components"]:
        blob = bundle / "blobs" / component["payload"]["sha256"]
        assert hashlib.sha256(blob.read_bytes()).hexdigest() == component["payload"]["sha256"]
        with tarfile.open(blob, "r:") as archive:
            members = archive.getmembers()
            assert [member.name for member in members] == sorted(member.name for member in members)
            for member in members:
                assert member.uid == member.gid == member.mtime == 0
                assert member.mode & ~0o777 == 0
                assert member.isfile() or member.isdir() or member.issym()
                assert not Path(member.name).is_absolute() and ".." not in Path(member.name).parts
    original = {path.relative_to(source).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in source.rglob("*") if path.is_file()}
    digest = hashlib.sha256(payload).hexdigest()
    shutil.rmtree(producer)
    assert not producer.exists()
    return bundle, public_key, digest, original, manifest


def imported(env, bundle, digest):
    facts = json.loads(package(env, "import", "--bundle", bundle).stdout)
    assert facts["manifest"] == digest
    inspected = json.loads(package(env, "inspect", "--manifest", digest).stdout)
    assert inspected["manifest"] == digest
    return Path(facts["payload"])


def assert_native(root, env):
    native = root / "components/native"
    probe = native / "bin/static"
    result = checked([probe, "argument with spaces", "", "λ"], env=env, text=True)
    assert result.stdout == "20:argument with spaces\n0:\n2:λ\n"
    assert subprocess.run([probe, "exit"], env=env, timeout=10).returncode == 37
    assert subprocess.run([probe, "signal"], env=env, timeout=10).returncode == -signal.SIGTERM
    launcher = native / "bin/launch"
    result = checked([launcher, "dynamic"], env=env, text=True)
    assert result.stdout == "transitive=42 plugin=73 resource=portable\n"
    result = checked([launcher, "interpreter", "child", "argument with spaces", ""], env=env, text=True)
    assert result.stdout == "5:child\n20:argument with spaces\n0:\n"
    assert subprocess.run([launcher, "interpreter", "exit"], env=env, timeout=10).returncode == 37
    assert subprocess.run([launcher, "interpreter", "signal"], env=env, timeout=10).returncode == -signal.SIGTERM
    alias = Path(env["HOME"]) / "symlink entry"
    alias.symlink_to(launcher)
    assert checked([alias, "dynamic"], env=env, text=True).stdout == (
        "transitive=42 plugin=73 resource=portable\n")
    alias.unlink()


def test_native_independent_roots_export_and_passive_import(sandbox, published):
    bundle, public_key, digest, original, manifest = published
    roots = []
    for name in ("recipient-one", "recipient two with spaces and a longer prefix"):
        env = recipient(sandbox / name)
        trust(env, public_key)
        root = imported(env, bundle, digest)
        roots.append(root)
        assert root.is_relative_to(Path(env["GRIPSACK_HOME"]))
        assert not (Path(env["HOME"]) / ".profile").exists()
        assert not (Path(env["HOME"]) / ".config/systemd/user").exists()
        assert "SHARING_PASSIVE" not in env
        for relative, expected in original.items():
            path = root / relative
            assert hashlib.sha256(path.read_bytes()).hexdigest() == expected
        assert (root / "components/native/bin/static").stat().st_mode & 0o111 == 0o111
        assert_native(root, env)
        exported = Path(env["HOME"]) / "exported"
        package(env, "export", "--manifest", digest, "--output", exported)
        assert (exported / "manifest.json").read_bytes() == (bundle / "manifest.json").read_bytes()
        for blob in (bundle / "blobs").iterdir():
            assert (exported / "blobs" / blob.name).read_bytes() == blob.read_bytes()
        # Copying here tests only this explicit relocatable launcher contract.
        moved = Path(env["HOME"]) / "moved runtime with spaces"
        shutil.copytree(root, moved, symlinks=True)
        assert_native(moved, env)
    assert roots[0] != roots[1]
    assert manifest["environment"] == {"SHARING_PASSIVE": {"kind": "literal", "value": "not-installed-by-import"}}


@pytest.mark.parametrize("damage", ["blob", "metadata", "revoked", "unknown-authority"])
def test_import_rejects_tampering_and_revoked_keys(sandbox, published, damage):
    bundle, public_key, digest, _, _ = published
    env = recipient(sandbox / "recipient")
    trust(env, public_key, revoked=damage == "revoked")
    altered = sandbox / "altered"
    shutil.copytree(bundle, altered)
    if damage == "blob":
        blob = next((altered / "blobs").iterdir())
        data = bytearray(blob.read_bytes())
        data[len(data) // 2] ^= 1
        blob.write_bytes(data)
    elif damage in ("metadata", "unknown-authority"):
        path = altered / "manifest.json"
        manifest = json.loads(path.read_bytes())
        if damage == "metadata":
            manifest["environment"]["SHARING_PASSIVE"]["value"] = "tampered"
        else:
            manifest["tasks"] = [{"command": "touch should-never-run"}]
        path.write_bytes(canonical(manifest))
    package(env, "import", "--bundle", altered, ok=False)
    package(env, "inspect", "--manifest", digest, ok=False)
    assert not (Path(env["HOME"]) / "should-never-run").exists()


def test_current_revocation_blocks_existing_receipt(sandbox, published):
    bundle, public_key, digest, _, _ = published
    env = recipient(sandbox / "recipient")
    trust(env, public_key)
    root = imported(env, bundle, digest)
    trust(env, public_key, revoked=True)
    package(env, "inspect", "--manifest", digest, ok=False)
    package(env, "export", "--manifest", digest, "--output", sandbox / "rejected-export", ok=False)
    assert root.exists(), "revocation must not destroy retained payload bytes"


@pytest.mark.native_integration
def test_registry_roundtrip_requires_explicit_local_registry(sandbox, published):
    url = os.environ.get("GRIPSACK_TEST_PACKAGE_REGISTRY")
    assert url, (
        "real registry qualification requires GRIPSACK_TEST_PACKAGE_REGISTRY=http://127.0.0.1:PORT; "
        "run offline suite with -k 'not registry' when no external registry fixture is provided"
    )
    from urllib.parse import urlparse
    parsed = urlparse(url)
    assert parsed.scheme == "http" and parsed.hostname in {"127.0.0.1", "::1", "localhost"}
    bundle, public_key, digest, _, _ = published
    sender = recipient(sandbox / "registry-sender")
    trust(sender, public_key)
    imported(sender, bundle, digest)
    repository = "gripsack-sharing/" + digest
    pushed = json.loads(package(sender, "push", "--manifest", digest, "--registry", url,
                                "--repository", repository, "--reference", "fixture",
                                "--insecure-loopback").stdout)
    receiver = recipient(sandbox / "registry recipient")
    trust(receiver, public_key)
    sender_home = Path(sender["HOME"])
    # Imported store directories are immutable; remove only this disposable
    # sender's own tree so the receiving process cannot reach its cache/store.
    for directory in [sender_home, *sender_home.rglob("*")]:
        if directory.is_dir() and not directory.is_symlink():
            directory.chmod(0o700)
    shutil.rmtree(sender_home)
    shutil.rmtree(bundle)
    package(receiver, "pull", "--digest", pushed["transport_digest"], "--registry", url,
            "--repository", repository, "--insecure-loopback")
    inspected = json.loads(package(receiver, "inspect", "--manifest", digest).stdout)
    assert_native(Path(inspected["payload"]), receiver)


def test_project_consumes_frozen_package_without_producer_or_bundle(sandbox, published):
    bundle, public_key, digest, _, _ = published
    env = recipient(sandbox / "project recipient")
    trust(env, public_key)
    imported(env, bundle, digest)
    shutil.rmtree(bundle)
    repo = make_workspace_repo(Path(env["HOME"]) / "project", '''
        import { pkg, provider, source, environment } from "@gripsack/core";
        const target = {os:"linux",arch:"x86_64"};
        export default [
          pkg("shared", {
            producer: provider(source.package({manifest:''' + json.dumps(digest) + '''})),
            commands: {probe:"components/native/bin/static"},
            target, layout:{kind:"relocatable"},
          }),
          environment("dev", {packages:["shared"],target}),
        ];
    ''')

    def project(*args, ok=True):
        result = run_grip([str(GRIP.resolve()), *args], cwd=repo, env=env,
                          capture_output=True, text=True, timeout=120)
        assert (result.returncode == 0) == ok, result.stdout + result.stderr
        return result

    project("update")
    lock = (repo / "gripsack.lock").read_bytes()
    assert digest.encode() in lock
    result = project("run", "--env", "dev", "--", "probe", "recipient", "")
    assert result.stdout == "9:recipient\n0:\n"
    assert (repo / "gripsack.lock").read_bytes() == lock
    assert not (Path(env["GRIPSACK_HOME"]) / "current").exists()
    assert not (Path(env["GRIPSACK_HOME"]) / "buildkit").exists()
    trust(env, public_key, revoked=True)
    project("run", "--env", "dev", "--", "probe", "revoked", ok=False)
