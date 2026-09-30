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
- fixture env repos use the TypeScript frontend under the defineEnv
  contract (plan/0013 D5): each modules/<name>.ts default-exports its
  module value — module() constructs, it never registers — and
  hosts/<host>.ts imports the modules it wants and returns them from
  defineEnv. Falsy entries drop out; that is how tests undeclare.
"""

from __future__ import annotations

import io
import json
import os
import stat
import re
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


def make_tarball(path: Path, files: dict[str, bytes]) -> Path:
    """Build a fixture tarball: module payload without the network."""
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
    with tarfile.open(path, "w:gz") as tar:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            info.mode = 0o755
            tar.addfile(info, io.BytesIO(content))
    return path


def refresh_host(repo: Path, host: str = "testhost") -> Path:
    """(Re)write hosts/<host>.ts importing every modules/*.ts — the
    defineEnv contract. Deterministic: files are globbed sorted, so the
    emitted module order (and the IR) is stable."""
    mods = sorted((repo / "modules").glob("*.ts")) if (repo / "modules").is_dir() else []
    # hyphens are legal filenames but not JS identifiers
    ident = lambda stem: re.sub(r"\W", "_", stem)  # noqa: E731
    imports = "".join(
        f'import {ident(p.stem)} from "../modules/{p.stem}.ts";\n' for p in mods
    )
    listing = ", ".join(ident(p.stem) for p in mods)
    (repo / "hosts").mkdir(exist_ok=True)
    (repo / "hosts" / f"{host}.ts").write_text(
        f'import {{ defineEnv }} from "@gripsack/core";\n'
        f"{imports}"
        f"\n"
        f"export default defineEnv((ctx) => ({{\n"
        f'  tags: ["test"],\n'
        f"  modules: [{listing}],\n"
        f"}}));\n"
    )
    return repo


def make_env_repo(
    root: Path,
    modules: dict[str, str] | str,
    host: str = "testhost",
) -> Path:
    """A fixture env repo mirroring plan/0001 §5, TypeScript frontend.

    `modules` maps a file basename under modules/ to file content; a
    bare string is shorthand for {"hello": ...}. Each file must
    default-export its module value (or an array of them)."""
    if isinstance(modules, str):
        modules = {"hello": modules}
    (root / "modules").mkdir(parents=True)
    (root / "hosts").mkdir()
    (root / "env.toml").write_text('[env]\nname = "fixture"\n')
    for name, ts in modules.items():
        (root / "modules" / f"{name}.ts").write_text(ts)
    return refresh_host(root, host=host)


def remove_module(repo: Path, name: str, host: str = "testhost") -> None:
    """Undeclare a module: delete its file and drop it from the host
    entrypoint (registration is explicit now — an unimported file is
    inert, but keeping the host honest is the point)."""
    (repo / "modules" / f"{name}.ts").unlink(missing_ok=True)
    refresh_host(repo, host=host)



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
    if arguments[0] not in {"check", "plan", "apply", "update", "adopt"} or "--ir" in arguments:
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
