"""E2E harness (plan/0003 §5, .agents/skills/gripsack-e2e).

Rules:
- everything under tmp_path; HOME and GRIPSACK_HOME are redirected there —
  a test can never touch the developer's real profile;
- disposable reviewed fixtures use explicit source/policy digest approval;
  gate tests disable fixture approval rather than an application bypass;
- offline only: sources are file:// fixture tarballs built here, or local
  git repos. Network in e2e is a bug;
- the binary comes from GRIPSACK_BIN (set in docker; the gate stage has
  already compiled it — never rebuild from e2e);
- fixture repositories have a root gripsack.ts importing default workspace
  values from outputs/*.ts and adopted profiles/*.ts in deterministic order;
  duplicate basenames are rejected before root rewriting or declaration removal.
"""

from __future__ import annotations

import io
import json
import os
import platform
import stat
import subprocess
import shutil
import sys
import tarfile
from pathlib import Path
from urllib.parse import unquote, urlparse

import pytest

# repo-root-relative default: works from e2e/ (pytest) and the repo
# root (manual runs) alike — a cwd-relative default broke the macOS
# CI job, which runs pytest under e2e/
_REPO_ROOT = Path(__file__).resolve().parent.parent
GRIP = Path(os.environ.get("GRIPSACK_BIN", str(_REPO_ROOT / "target/debug/grip")))
_fixture_root: Path | None = None


def native_target() -> str:
    """A TypeScript object literal matching this offline fixture's native host."""
    machine = platform.machine()
    target = {
        "os": "macos" if sys.platform == "darwin" else "linux",
        "arch": {"arm64": "aarch64", "AMD64": "x86_64"}.get(machine, machine),
    }
    if target["os"] == "linux":
        target["abi"] = "musl" if platform.libc_ver()[0] == "musl" else "gnu"
    return json.dumps(target)


def make_tarball(path: Path, files: dict[str, bytes]) -> Path:
    """Build a fixture package payload without the network."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(path, "w:gz") as tar:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            tar.addfile(info, io.BytesIO(content))
    return path


def make_toolchain_tarball(path: Path, files: dict[str, bytes]) -> Path:
    """A fixture tarball with EXECUTABLE members — `which <bin>`
    inside a build step needs the exec bit (0039 fixtures); the plain
    helper above is data-only by default."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(path, "w:gz") as tar:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            info.mode = 0o755
            tar.addfile(info, io.BytesIO(content))
    return path


def workspace_output_files(repo: Path) -> list[Path]:
    """Discover authored and adopted outputs, refusing ambiguous basenames."""
    outputs = sorted(
        path
        for directory in ("outputs", "profiles")
        for path in (repo / directory).glob("*.ts")
    )
    names = set()
    for path in outputs:
        if path.stem in names:
            raise ValueError(f"duplicate workspace output basename: {path.stem}")
        names.add(path.stem)
    return outputs


def refresh_workspace(repo: Path) -> Path:
    """Rebuild root imports from current authored and adopted output values."""
    outputs = workspace_output_files(repo)
    imports = "".join(
        f'import output_{index} from "./{path.relative_to(repo).as_posix()}";\n'
        for index, path in enumerate(outputs)
    )
    listing = "".join(f"    output_{index},\n" for index in range(len(outputs)))
    (repo / "gripsack.ts").write_text(
        'import { defineWorkspace, workspace } from "@gripsack/core";\n'
        + imports
        + "export default defineWorkspace(() => workspace({\n"
        + "  outputs: [\n"
        + listing
        + "  ].flat(),\n"
        + "}));\n"
    )
    return repo


def make_workspace_repo(root: Path, outputs: dict[str, str] | str) -> Path:
    """Write real workspace output declarations, never translate legacy DSL."""
    if isinstance(outputs, str):
        outputs = {"hello": outputs}
    (root / "outputs").mkdir(parents=True, exist_ok=True)
    (root / "env.toml").write_text('[env]\nname = "fixture"\n')
    for name, source in outputs.items():
        (root / "outputs" / f"{name}.ts").write_text(source)
    return refresh_workspace(root)


def remove_output(repo: Path, name: str) -> None:
    """Remove the declaration and its explicit root import."""
    for path in workspace_output_files(repo):
        if path.stem == name:
            path.unlink()
    refresh_workspace(repo)



@pytest.fixture(scope="session")
def confined_runtime(tmp_path_factory) -> Path:
    """One session copy of the real evaluator runtime, reachable on PATH.

    Kernel confinement grants reads only along the operator's executable
    space, so wrapper-style GRIPSACK_DENO fixtures must exec their real
    runtime through PATH, not through an arbitrary absolute path."""
    deno = Path(os.environ["GRIPSACK_DENO"]).resolve()
    directory = tmp_path_factory.mktemp("confined-runtime")
    target = directory / "deno"
    shutil.copy2(deno, target)
    target.chmod(0o755)
    return directory

@pytest.fixture
def sandbox(tmp_path, monkeypatch, request, confined_runtime):
    """Redirect everything gripsack touches into tmp_path. On test
    failure, the grip run log (JSONL with causal spans — the debug
    skill's first stop) is printed before the sandbox evaporates:
    bazel keeps per-test logs for the same reason, and CI failures
    on a platform you can't run locally are otherwise archaeology."""
    monkeypatch.setenv("PATH", f"{confined_runtime}{os.pathsep}{os.environ.get('PATH', '')}")
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("GRIPSACK_HOME", str(tmp_path / ".local/share/gripsack"))
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)
    monkeypatch.delenv("GRIPSACK_TRUST_ALL", raising=False)
    monkeypatch.setattr(sys.modules[__name__], "_fixture_root", tmp_path.resolve())
    yield tmp_path
    rep = getattr(request.node, "rep_call", None)
    if rep is not None and rep.failed:
        runs = tmp_path / ".local/share/gripsack/runs"
        log = latest_run_log(runs)
        if log:
            tail = log.read_text(errors="replace").splitlines()[-25:]
            print(
                "\n--- grip run log ({}), last 25 lines ---\n{}".format(
                    log.name, "\n".join(tail)
                ),
                file=sys.stderr,
            )


def latest_run_log(runs: Path) -> Path | None:
    """The explicit pointer wins; mtime is only a broken-pointer fallback."""
    try:
        target = (runs / "latest").resolve(strict=True)
        if target.parent == runs.resolve() and target.suffix == ".jsonl" and target.is_file():
            return target
    except (OSError, RuntimeError):
        pass
    candidates = []
    if runs.is_dir():
        for path in runs.glob("*.jsonl"):
            try:
                metadata = path.lstat()
                if stat.S_ISREG(metadata.st_mode):
                    candidates.append((metadata.st_mtime_ns, path.name, path))
            except OSError:
                continue
    return max(candidates)[2] if candidates else None


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    """Stash the per-phase result on the node so the sandbox fixture
    can react to the call phase's outcome (see sandbox)."""
    outcome = yield
    rep = outcome.get_result()
    setattr(item, f"rep_{rep.when}", rep)


def approve_fixture(command, *, cwd=None, env=None):
    """Explicitly approve only reviewed disposable fixtures, never real HOME.

    Inspection does not evaluate source. Its exact two fingerprints are passed
    to add; a concurrent source/policy change is a failure, not a blanket grant.
    Fault-injection controls belong to the tested command, not fixture setup.
    """
    if _fixture_root is None or len(command) < 2:
        return
    arguments = list(map(str, command[1:]))
    if arguments[0] not in {"check", "plan", "apply", "update", "adopt", "build", "run", "shell", "task"} or "--ir" in arguments:
        return
    environment = dict(os.environ if env is None else env)
    home = Path(environment["HOME"]).resolve()
    state = Path(environment.get("GRIPSACK_HOME", str(home / ".local/share/gripsack"))).resolve()
    assert home.is_relative_to(_fixture_root) and state.is_relative_to(_fixture_root)
    specification = None
    for index, argument in enumerate(arguments):
        if argument == "--repo" and index + 1 < len(arguments):
            specification = arguments[index + 1]
        elif argument.startswith("--repo="):
            specification = argument.removeprefix("--repo=")
    root = Path(cwd or Path.cwd()).resolve()
    if specification is None:
        specification = str(root)
        assert root.is_relative_to(_fixture_root)
    elif "://" in specification:
        parsed = urlparse(specification)
        assert parsed.scheme == "file" and parsed.netloc in {"", "localhost"}, "fixture repo cloning must remain offline"
        assert Path(unquote(parsed.path)).resolve().is_relative_to(_fixture_root)
    else:
        candidate = Path(specification)
        candidate = candidate if candidate.is_absolute() else root / candidate
        assert candidate.resolve().is_relative_to(_fixture_root)
    for key in list(environment):
        if key.startswith(("GRIPSACK_FS_", "GRIPSACK_CRASH_", "GRIPSACK_EVAL_PAUSE_")):
            environment.pop(key)
    inspect = subprocess.run(
        [str(command[0]), "trust", "inspect", specification, "--json"],
        cwd=cwd, env=environment, capture_output=True, text=True, timeout=120,
    )
    if inspect.returncode:
        # Invalid config/source/runtime cases must reach the tested command's
        # actual diagnostic, not turn setup into a successful authorization.
        return
    inspected = json.loads(inspect.stdout)
    assert Path(inspected["repository"]).is_relative_to(_fixture_root)
    if inspected["approval"]["status"]["status"] == "approved":
        return
    added = subprocess.run(
        [str(command[0]), "trust", "add", specification,
         "--bundle", inspected["bundle_digest"], "--policy", inspected["policy_digest"]],
        cwd=cwd, env=environment, capture_output=True, text=True, timeout=120,
    )
    assert added.returncode == 0, added.stdout + added.stderr


def run_grip(command, *, cwd=None, env=None, approve=True, **options):
    if approve:
        approve_fixture(command, cwd=cwd, env=env)
    return subprocess.run(command, cwd=cwd, env=env, **options)


def start_grip(command, *, cwd=None, env=None, approve=True, **options):
    if approve:
        approve_fixture(command, cwd=cwd, env=env)
    return subprocess.Popen(command, cwd=cwd, env=env, **options)


def grip(*args: str, cwd: Path | None = None, approve: bool = True) -> subprocess.CompletedProcess:
    assert GRIP.exists(), f"grip binary not found at {GRIP} (build first)"
    return run_grip(
        [str(GRIP.resolve()), *args], cwd=cwd, approve=approve,
        capture_output=True, text=True, timeout=120,
    )

def _seed_plugin_store(sandbox, exe, fixture, tag="1.0"):
    """Pre-seed the managed plugin store as if a prior provision ran."""
    home = sandbox / ".local/share/gripsack"
    bindir = home / "plugins" / exe / tag
    bindir.mkdir(parents=True)
    (bindir / exe).write_text(fixture)
    (bindir / exe).chmod(0o755)
    (home / "plugins" / exe / "current").symlink_to(f"{tag}/")
    (home / "plugins" / "receipts").mkdir(parents=True)
    (home / "plugins" / "receipts" / f"{exe}.toml").write_text(
        f'source = "acme/{exe}"\ntag = "{tag}"\nsha256 = "ab"\n'
    )


def only_store_path(sandbox):
    store = sandbox / ".local/share/gripsack/store"
    entries = [p.name for p in store.iterdir()]
    assert len(entries) == 1, f"expected one store path, found {entries}"
    return entries[0]
