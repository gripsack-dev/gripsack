"""Real ABI-tagged extension wheels in a frozen offline Conda environment.

Preparation is explicit and separate from pytest: it acquires the existing
pinned runtime fixture, imports it, and compiles this test-owned C source against
that exact Conda Python's headers. No production source builder or package-owned
setup script is involved. Tests consume only original archives and the pinned
helper, with network disabled and distinct recipient homes.
"""
from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import zipfile

import pytest

from test_package_conda_offline import (
    offline_recipient, prepare_runtime_fixture, remove_producer, sha, workspace_command,
)
from test_package_sharing import canonical, checked, independent_sign, package

pytestmark = [
    pytest.mark.native_integration,
    pytest.mark.skipif(
        platform.system() != "Linux" or platform.machine() != "x86_64",
        reason="native Linux x86_64 wheel ABI qualification",
    ),
]

NAME = "gripsack-native-wheel"
DIST_INFO = "gripsack_native_wheel-1.0.dist-info"
EXTENSION = "gripsack_native_wheel/_native.abi3.so"
TAG = "cp38-abi3-manylinux_2_17_x86_64"
WHEEL_MEDIA = "application/vnd.gripsack.python.wheel.v1"

EXTENSION_SOURCE = r'''
#define Py_LIMITED_API 0x03080000
#include <Python.h>

static PyObject *answer(PyObject *self, PyObject *unused) {
    (void)self;
    (void)unused;
    return PyLong_FromLong(73);
}

static PyMethodDef methods[] = {
    {"answer", answer, METH_NOARGS, "Return a value from the real native extension."},
    {NULL, NULL, 0, NULL}
};

static struct PyModuleDef module = {
    PyModuleDef_HEAD_INIT,
    "gripsack_native_wheel._native",
    NULL,
    -1,
    methods,
    NULL,
    NULL,
    NULL,
    NULL
};

PyMODINIT_FUNC PyInit__native(void) {
    return PyModule_Create(&module);
}
'''

PYTHON_MODULE = b'''import json
import sys
from . import _native

def main():
    print(json.dumps({"answer": _native.answer(), "extension": _native.__file__, "prefix": sys.prefix}))
'''


def authority(directory):
    directory.mkdir()
    private = directory / "private.pem"
    public_der = directory / "public.der"
    checked(["openssl", "genpkey", "-algorithm", "ED25519", "-out", private])
    checked(["openssl", "pkey", "-in", private, "-pubout", "-outform", "DER", "-out", public_der])
    return private, base64.b64encode(public_der.read_bytes()[-32:]).decode()


def write_wheel(directory, extension, tag=TAG):
    """Produce an ordinary binary wheel, hashing every original RECORD member."""
    files = {
        EXTENSION: extension,
        "gripsack_native_wheel/__init__.py": PYTHON_MODULE,
        f"{DIST_INFO}/METADATA": (
            b"Metadata-Version: 2.3\nName: gripsack-native-wheel\nVersion: 1.0\n"
            b"Requires-Python: >=3.8\n\nA real stable-ABI acceptance extension.\n"
        ),
        f"{DIST_INFO}/WHEEL": (
            "Wheel-Version: 1.0\nGenerator: gripsack-native-acceptance\n"
            f"Root-Is-Purelib: false\nTag: {tag}\n"
        ).encode(),
        f"{DIST_INFO}/entry_points.txt": (
            b"[console_scripts]\nnative-wheel = gripsack_native_wheel:main\n"
        ),
    }
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\n")
    for name, data in sorted(files.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        writer.writerow([name, f"sha256={digest}", len(data)])
    writer.writerow([f"{DIST_INFO}/RECORD", "", ""])
    files[f"{DIST_INFO}/RECORD"] = record.getvalue().encode()
    filename = f"gripsack_native_wheel-1.0-{tag}.whl"
    wheel = directory / filename
    with zipfile.ZipFile(wheel, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in sorted(files.items()):
            member = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            member.compress_type = zipfile.ZIP_DEFLATED
            member.external_attr = 0o100644 << 16
            archive.writestr(member, data)
    # Field order is the production LockedWheel serde order. Empty dependency
    # metadata is omitted, exactly as the persisted Rust representation does.
    locked = {
        "name": NAME, "version": "1.0", "filename": filename,
        "url": f"https://fixture.invalid/{filename}", "sha256": sha(wheel),
        "size": wheel.stat().st_size, "requires_python": ">=3.8",
    }
    return wheel, locked


def add_native_wheel(bundle, wheel, locked):
    """Extend a captured complete lock; never resolve or fetch a dependency."""
    manifest = json.loads((bundle / "manifest.json").read_bytes())
    portable = manifest["components"][0]["conda"]
    closure = portable["lock"]
    previous = next((item for item in closure.get("wheels", []) if item["name"] == NAME), None)
    closure["wheels"] = sorted(
        [item for item in closure.get("wheels", []) if item["name"] != NAME] + [locked],
        key=lambda item: item["name"],
    )
    requirements = closure["system_requirements"]
    virtuals = [dict(item) for item in requirements.get("virtual_packages", [])]
    glibc = next((item for item in virtuals if item["name"] == "__glibc"), None)
    if glibc is None:
        virtuals.append({"name": "__glibc", "minimum_version": "2.17"})
    else:
        current = tuple(map(int, glibc["minimum_version"].split(".")[:2]))
        if current < (2, 17):
            glibc["minimum_version"] = "2.17"
    closure["system_requirements"] = {
        "virtual_packages": sorted(virtuals, key=lambda item: item["name"]),
        **({"archspec": requirements["archspec"]} if "archspec" in requirements else {}),
    }
    if previous:
        portable["archives"] = [item for item in portable["archives"] if item["sha256"] != previous["sha256"]]
        (bundle / "blobs" / previous["sha256"]).unlink()
    descriptor = {"sha256": locked["sha256"], "size": locked["size"], "media_type": WHEEL_MEDIA}
    portable["archives"] = sorted([*portable["archives"], descriptor], key=lambda item: item["sha256"])
    shutil.copyfile(wheel, bundle / "blobs" / locked["sha256"])
    manifest["commands"]["native-wheel"] = {
        "component": manifest["components"][0]["id"], "executable": "bin/native-wheel", "args": [],
    }
    manifest["commands"] = dict(sorted(manifest["commands"].items()))
    (bundle / "manifest.json").write_bytes(canonical(manifest))
    return manifest


def fixture_bundle(sandbox):
    source = os.environ.get("GRIPSACK_TEST_NATIVE_WHEEL_BUNDLE")
    assert source, (
        "required real native wheel fixture is absent; explicitly prepare with "
        "python e2e/test_package_native_wheel.py prepare --directory /tmp/native-wheel-inputs "
        "[--runtime-bundle /tmp/conda-runtime-inputs/bundle], then set "
        "GRIPSACK_TEST_NATIVE_WHEEL_BUNDLE=/tmp/native-wheel-inputs/bundle"
    )
    source = Path(source).resolve()
    assert source.is_dir(), f"required native wheel bundle is absent: {source}"
    bundle = sandbox / "native wheel transfer"
    shutil.copytree(source, bundle)
    manifest = json.loads((bundle / "manifest.json").read_bytes())
    portable = manifest["components"][0]["conda"]
    locked = next(item for item in portable["lock"]["wheels"] if item["name"] == NAME)
    assert locked["filename"].endswith(f"-{TAG}.whl")
    assert sha(bundle / "blobs" / locked["sha256"]) == locked["sha256"]
    with zipfile.ZipFile(bundle / "blobs" / locked["sha256"]) as archive:
        extension = archive.read(EXTENSION)
        assert archive.read(f"{DIST_INFO}/WHEEL").find(f"Tag: {TAG}\n".encode()) >= 0
    assert extension[:6] == b"\x7fELF\x02\x01" and int.from_bytes(extension[18:20], "little") == 62
    return bundle, manifest, locked, extension


def test_real_abi3_wheel_executes_after_offline_rematerialization_and_cache_loss(sandbox):
    bundle, manifest, locked, original_extension = fixture_bundle(sandbox)
    signing = sandbox / "native wheel authority"
    private, public = authority(signing)
    independent_sign(bundle, private, signing)
    identity = sha(bundle / "manifest.json")
    recipients = []
    for name in ("native wheel recipient one", "native wheel recipient " + "long prefix " * 4):
        env = offline_recipient(sandbox / name, public)
        imported = json.loads(package(env, "import", "--bundle", bundle).stdout)
        assert imported["manifest"] == identity
        prefix = Path(imported["payload"])
        assert prefix.is_relative_to(Path(env["GRIPSACK_HOME"]))
        recipients.append((env, prefix))
    assert recipients[0][1] != recipients[1][1]
    shutil.rmtree(bundle)
    shutil.rmtree(signing)
    for env, prefix in recipients:
        workspace_command(env, Path(env["HOME"]), "gc")
        before = {str(path.relative_to(prefix)): sha(path) for path in prefix.rglob("*")
                  if path.is_file() and not path.is_symlink()}
        result = checked([prefix / "bin/native-wheel"],
                         env=dict(env, PYTHONDONTWRITEBYTECODE="1"), text=True)
        observed = json.loads(result.stdout)
        assert observed["answer"] == 73
        assert Path(observed["prefix"]) == prefix
        extension = Path(observed["extension"])
        assert extension.is_relative_to(prefix) and extension.name == "_native.abi3.so"
        assert extension.read_bytes() == original_extension
        direct = checked([prefix / "bin/python", "-I", "-B", "-c",
                          "from gripsack_native_wheel._native import answer; print(answer())"],
                         env=env, text=True)
        assert direct.stdout.strip() == "73"
        receipt = json.loads((prefix / f"conda-meta/{NAME}-1.0-wheel.json").read_bytes())
        assert receipt["wheel"] == locked and receipt["final_prefix"] == str(prefix)
        assert str(extension.relative_to(prefix)) in receipt["files"]
        assert "bin/native-wheel" in receipt["files"]
        package(env, "inspect", "--manifest", identity)
        after = {str(path.relative_to(prefix)): sha(path) for path in prefix.rglob("*")
                 if path.is_file() and not path.is_symlink()}
        assert before == after, "native wheel runtime modified its immutable environment"
        retained = {sha(path) for path in (Path(env["GRIPSACK_HOME"]) / "store").rglob("archive")}
        assert locked["sha256"] in retained
        assert not list(prefix.rglob("*.pyc"))


def test_overlong_binary_prefix_refuses_without_publishing_an_environment(sandbox):
    bundle, _, _, _ = fixture_bundle(sandbox)
    signing = sandbox / "prefix authority"
    private, public = authority(signing)
    independent_sign(bundle, private, signing)
    identity = sha(bundle / "manifest.json")
    # Preserve the originally exercised overlong recipient as a refusal case.
    # Conda binary placeholders have finite capacity; reconstruction cannot grow them.
    env = offline_recipient(sandbox / ("native wheel recipient " + "long prefix " * 10), public)
    rejected = package(env, "import", "--bundle", bundle, ok=False)
    assert rejected.returncode == 1
    assert package(env, "inspect", "--manifest", identity, ok=False).returncode == 1
    store = Path(env["GRIPSACK_HOME"]) / "store"
    assert not list(store.rglob("bin/native-wheel"))
    assert not list(store.rglob("_native.abi3.so"))
    assert not (Path(env["GRIPSACK_HOME"]) / "current").exists()


@pytest.mark.parametrize("incompatible_tag", [
    "cp313-cp313-manylinux_2_17_x86_64",
    "cp312-cp311-manylinux_2_17_x86_64",
    "cp38-abi3-manylinux_2_17_aarch64",
    "cp38-abi3-musllinux_1_2_x86_64",
])
def test_native_wheel_tag_mismatch_refuses_before_prefix_effects(sandbox, incompatible_tag):
    bundle, manifest, _, extension = fixture_bundle(sandbox)
    python = next(item for item in manifest["components"][0]["conda"]["lock"]["packages"]
                  if item["name"] == "python")
    assert python["version"].split(".")[:2] == ["3", "12"], "fixture must retain the prepared CPython 3.12 closure"
    wheel, locked = write_wheel(sandbox, extension, incompatible_tag)
    add_native_wheel(bundle, wheel, locked)
    signing = sandbox / "negative authority"
    private, public = authority(signing)
    independent_sign(bundle, private, signing)
    env = offline_recipient(sandbox / "incompatible recipient", public)
    assert package(env, "import", "--bundle", bundle, ok=False).returncode == 1
    store = Path(env["GRIPSACK_HOME"]) / "store"
    assert not list(store.rglob("bin/python"))
    assert not list(store.rglob("_native.abi3.so"))
    assert not list(store.rglob(f"{NAME}-1.0-wheel.json"))
    assert not (Path(env["GRIPSACK_HOME"]) / "current").exists()


def prepare_native_fixture(directory, runtime_bundle=None):
    """Explicit preparation only; pytest never invokes acquisition or compilation."""
    directory = directory.resolve()
    assert not directory.exists(), "native fixture preparation requires a fresh directory"
    directory.mkdir(parents=True)
    if runtime_bundle is None:
        prepare_runtime_fixture(directory / "runtime-inputs")
        runtime_bundle = directory / "runtime-inputs" / "bundle"
    else:
        runtime_bundle = runtime_bundle.resolve()
    assert runtime_bundle.is_dir(), f"prepared Conda runtime bundle absent: {runtime_bundle}"
    base = directory / "runtime-transfer"
    shutil.copytree(runtime_bundle, base)
    signing = directory / "preparation authority"
    private, public = authority(signing)
    independent_sign(base, private, signing)
    env = offline_recipient(directory / "compiler runtime", public)
    prefix = Path(json.loads(package(env, "import", "--bundle", base).stdout)["payload"])
    manifest = json.loads((base / "manifest.json").read_bytes())
    python = next(item for item in manifest["components"][0]["conda"]["lock"]["packages"]
                  if item["name"] == "python")
    assert python["version"].split(".")[:2] == ["3", "12"], "preparation requires the real CPython 3.12 fixture"
    include = prefix / "include/python3.12"
    assert (include / "Python.h").is_file(), "retained Conda Python must provide its own C headers"
    compiler = shutil.which("cc")
    assert compiler, "explicit native-wheel preparation requires the native C compiler"
    build = directory / "fixture compilation"
    build.mkdir()
    source = build / "native.c"
    source.write_text(EXTENSION_SOURCE)
    extension = build / "_native.abi3.so"
    # No libc/CRT linkage, host Python headers, package setup scripts or ambient
    # CFLAGS. Only stable Python API symbols are resolved by the locked runtime.
    checked([compiler, "-shared", "-nostdlib", "-fPIC", "-O2", "-fno-stack-protector",
             "-fno-builtin", "-march=x86-64", "-mtune=generic", f"-I{include}",
             source, "-o", extension], env=env)
    wheel, locked = write_wheel(build, extension.read_bytes())
    bundle = directory / "bundle"
    shutil.copytree(base, bundle)
    add_native_wheel(bundle, wheel, locked)
    independent_sign(bundle, private, signing)
    remove_producer(Path(env["HOME"]))
    shutil.rmtree(base)
    shutil.rmtree(build)
    shutil.rmtree(signing)
    if (directory / "runtime-inputs").exists():
        shutil.rmtree(directory / "runtime-inputs")
    print(json.dumps({"bundle": str(bundle), "manifest": sha(bundle / "manifest.json"),
                      "wheel_sha256": locked["sha256"], "wheel_filename": locked["filename"]}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Explicitly provision a real offline native-wheel acceptance fixture")
    parser.add_argument("action", choices=["prepare"])
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--runtime-bundle", type=Path)
    arguments = parser.parse_args()
    prepare_native_fixture(arguments.directory, arguments.runtime_bundle)
