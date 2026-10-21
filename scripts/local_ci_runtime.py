"""Source-bound Docker handoffs for the reviewed-code, nonpublishing pilot.

The controller owns gate receipts; this module owns only actual project commands,
immutable image transport and exact container cleanup. BuildKit cache is retained,
never represented as a fresh execution of cached RUN instructions.
"""
from __future__ import annotations

import hashlib
import json
import platform
from pathlib import Path
import re
import shlex
import subprocess
import tempfile
import uuid

ROOT = Path(__file__).resolve().parent.parent
STATE = ROOT / "local-ci"
SOURCE_LABEL = "dev.gripsack.local-ci.source"
OWNER_LABEL = "dev.gripsack.local-ci.owner"


def run(argv, **kwargs):
    argv = [str(arg) for arg in argv]
    print("+ " + shlex.join(argv), flush=True)
    return subprocess.run(argv, check=True, cwd=ROOT, **kwargs)


def capture(argv):
    return run(argv, stdout=subprocess.PIPE, text=True).stdout.strip()


def source():
    if (platform.system(), platform.machine()) != ("Linux", "x86_64"):
        raise ValueError("local qualification requires native Linux/WSL x86_64")
    revision = capture(["git", "rev-parse", "HEAD"])
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("candidate must be a committed Git revision")
    # Restored declared artifacts are untracked, not candidate source changes.
    run(["git", "diff", "--exit-code", "HEAD", "--"])
    return revision


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def record(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def build_image(stage, *, export=False):
    directory = STATE / "images"
    directory.mkdir(parents=True, exist_ok=True)
    iid = directory / f"{stage}.iid"
    revision = source()
    contexts = STATE / "contexts"
    contexts.mkdir(exist_ok=True)
    # Docker sees only the frozen Git tree, never untracked/restored host files.
    with tempfile.TemporaryDirectory(dir=contexts, prefix=f"{stage}-") as temporary:
        context = Path(temporary)
        archive = context / "source.tar"
        run(["git", "-c", "tar.umask=0022", "archive", "--format=tar", "--output", archive, revision])
        # Do not extract under the worker's private umask: the data filter
        # intentionally drops directory modes, turning source directories into
        # root-owned 0700 paths after Docker COPY. Canonical Git modes retain
        # executable bits and match ordinary checkout contexts for cache reuse.
        with archive.open("rb") as context_input:
            run(["docker", "build", "--platform", "linux/amd64", "--progress=plain",
                 "--target", stage, "--label", f"{SOURCE_LABEL}={revision}",
                 "--iidfile", iid, "-"], stdin=context_input)
    image = iid.read_text().strip()
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", image):
        raise ValueError("Docker did not produce an immutable image ID")
    receipt = {"source": revision, "stage": stage, "image": image,
               "buildkit_cache": "retained; cached RUN steps are not fresh execution"}
    if export:
        archive = directory / f"{stage}.tar"
        run(["docker", "image", "save", "--output", archive, image])
        receipt["archive_sha256"] = digest(archive)
    record(directory / f"{stage}.json", receipt)
    return image


def load_image(stage):
    directory = STATE / "images"
    receipt = json.loads((directory / f"{stage}.json").read_text())
    archive = directory / f"{stage}.tar"
    if receipt["source"] != source() or receipt["stage"] != stage:
        raise ValueError("foreign source image receipt")
    if digest(archive) != receipt["archive_sha256"]:
        raise ValueError("image handoff digest mismatch")
    image = receipt["image"]
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", image):
        raise ValueError("invalid immutable image identity")
    run(["docker", "image", "load", "--input", archive])
    actual = json.loads(capture(["docker", "image", "inspect", image]))[0]
    if (actual["Id"] != image or actual["Os"] != "linux"
            or actual["Architecture"] != "amd64"
            or actual["Config"]["Labels"].get(SOURCE_LABEL) != receipt["source"]):
        raise ValueError("loaded image does not bind the required source/platform")
    return image


def container(image, command, *, mounts=(), environment=None, workdir=None,
              timeout=None, stdout=None, stderr=None, resources=(), user=None):
    owner = uuid.uuid4().hex
    directory = STATE / "containers"
    directory.mkdir(parents=True, exist_ok=True)
    cidfile = directory / f"{owner}.cid"
    argv = ["docker", "run", "--rm", "--pull=never", "--platform", "linux/amd64",
            "--name", f"gripsack-ci-{owner}", "--cidfile", cidfile,
            "--label", f"{OWNER_LABEL}={owner}",
            "--label", f"{SOURCE_LABEL}={source()}", *resources]
    for host, guest, readonly in mounts:
        argv += ["--mount", f"type=bind,src={Path(host).resolve()},dst={guest}" + (",readonly" if readonly else "")]
    for key, value in (environment or {}).items():
        argv += ["--env", f"{key}={value}"]
    argv += ["--env", "CARGO_BUILD_JOBS=4"]
    if workdir:
        argv += ["--workdir", workdir]
    if user is not None:
        argv += ["--user", user]
    try:
        return run([*argv, image, *command], timeout=timeout, stdout=stdout, stderr=stderr)
    finally:
        if cidfile.exists():
            identifier = cidfile.read_text().strip()
            if not re.fullmatch(r"[0-9a-f]{64}", identifier):
                raise ValueError("invalid owned container ID")
            inspected = subprocess.run(["docker", "inspect", identifier], capture_output=True, text=True)
            if inspected.returncode == 0:
                metadata = json.loads(inspected.stdout)[0]
                if metadata["Config"]["Labels"].get(OWNER_LABEL) != owner:
                    raise ValueError("container ownership changed; refusing cleanup")
                run(["docker", "rm", "--force", identifier])
            else:
                # Distinguish --rm completion from a daemon outage. Never claim
                # cleanup succeeded merely because inspect could not connect.
                run(["docker", "info", "--format", "{{.ServerVersion}}"])
