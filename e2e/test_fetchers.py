"""Fetcher e2e: tarball/git/plugin fetch specs, throttle, the managed
plugin store, and store dedup identities — split from test_flow.py;
fixture repos come from conftest."""



import os
import sys
import subprocess
import socket

import pytest

from conftest import (
    GRIP,
    _seed_plugin_store,
    grip,
    make_workspace_repo,
    make_tarball,
    run_grip,
)
from test_complete_pins import pins



def test_deferred_identity_is_stable_across_applies(sandbox):
    """Finding C: fetch kinds whose payload hash isn't knowable up
    front (git/plugin) must reuse immutable store objects and satisfy
    on the second apply, not a spurious second generation."""
    import subprocess as sp

    remote = sandbox / "remote"
    remote.mkdir()
    env = dict(
        GIT_AUTHOR_NAME="t",
        GIT_AUTHOR_EMAIL="t@t",
        GIT_COMMITTER_NAME="t",
        GIT_COMMITTER_EMAIL="t@t",
        PATH="/usr/bin:/bin:/usr/local/bin",
        HOME=str(sandbox),
    )
    sp.run(["git", "init", "--quiet"], cwd=remote, env=env, check=True)
    (remote / "bin.txt").write_text("payload\n")
    sp.run(["git", "add", "."], cwd=remote, env=env, check=True)
    sp.run(["git", "commit", "--quiet", "-m", "init"], cwd=remote, env=env, check=True)
    rev = sp.run(
        ["git", "rev-parse", "HEAD"], cwd=remote, env=env, check=True, capture_output=True, text=True
    ).stdout.strip()

    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ git, pkg, provider, profile, file, artifactFile, identity, symlinkTo }} from "@gripsack/core";

export default [
  pkg("tool", {{
    producer: provider(git("file://{remote}", "{rev}")), commands: {{}},
    target: {{ os: "linux", arch: "x86_64" }}, layout: {{ kind: "relocatable" }},
  }}),
  profile("tool-profile", {{ files: [file({{
    source: artifactFile("tool", "bin.txt"), content: identity(),
    destination: symlinkTo("~/.local/bin/tool"),
  }})] }}),
];
""",
    )
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "tool-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "applied" in out.stdout
    store = sandbox / ".local/share/gripsack/store"
    paths_after_first = sorted(p.name for p in store.iterdir())
    assert paths_after_first
    current = (sandbox / ".local/share/gripsack/current").readlink()

    out = grip("apply", "tool-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "already satisfied" in out.stdout, out.stdout
    assert sorted(p.name for p in store.iterdir()) == paths_after_first
    assert (sandbox / ".local/share/gripsack/current").readlink() == current


FETCH_FIXTURE = """#!/usr/bin/env python3
import json, os, sys
req = json.loads(sys.stdin.readline())
if req["op"] == "capabilities":
    print(json.dumps({"type": "response", "result": {
        "capabilities": {"throttle": {"demo.local": "1/s"}}}}))
elif req["op"] == "fetch":
    dest = req["dest_dir"]
    os.makedirs(os.path.join(dest, "bin"), exist_ok=True)
    with open(os.path.join(dest, "bin", "demo"), "w") as f:
        f.write("#!/bin/sh\\necho demo\\n")
    print(json.dumps({"type": "response", "result": {}}))
sys.stdout.flush()
"""


PLUGIN_OUTPUTS = {
    name: """
import { pluginFetch, pkg, provider, profile, file, artifactFile, identity, symlinkTo } from "@gripsack/core";

export default [
  pkg("%s", {
    producer: provider(pluginFetch("demo")), commands: {},
    target: { os: "linux", arch: "x86_64" }, layout: { kind: "relocatable" },
  }),
  profile("%s-profile", { files: [file({
    source: artifactFile("%s", "bin/demo"), content: identity(),
    destination: symlinkTo("~/.local/bin/demo-%s"),
  })] }),
];
"""
    % (name, name, name, suffix)
    for name, suffix in (("a", "a"), ("b", "b"))
}



def test_repo_build_env_reaches_plugin_discovery_and_both_child_operations(sandbox):
    """Repo PATH selects only artifact fetchers; capabilities and fetch see
    the same declared child variables without mutating grip's environment."""
    bindir = sandbox / "repo-tools"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(
        FETCH_FIXTURE.replace(
            "req = json.loads(sys.stdin.readline())",
            'if os.environ.get("REPO_FETCH_SENTINEL") != "on": sys.exit(24)\n'
            "req = json.loads(sys.stdin.readline())",
        )
    )
    fetcher.chmod(0o755)
    repo = make_workspace_repo(sandbox / "myenv", {"a": PLUGIN_OUTPUTS["a"]})
    with (repo / "env.toml").open("a") as output:
        output.write(
            f'\n[eval]\nenv = {{ PATH = "{bindir}:{os.environ["PATH"]}", '
            'REPO_FETCH_SENTINEL = "on" }\n'
        )

    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "a-profile", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()

def test_throttle_token_bucket_serializes_plugin_fetches(sandbox, monkeypatch):
    """[throttle] (0002): a fetcher declares its rate budget via the
    capabilities op; the core's token bucket enforces it across
    selected packages — two fetches at 1/s take >= ~1s wall."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    repo = make_workspace_repo(sandbox / "myenv", PLUGIN_OUTPUTS)
    import time

    start = time.monotonic()
    out = grip("update", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed >= 0.9, f"two 1/s fetches finished in {elapsed:.2f}s — bucket not enforced"
    applied = grip("apply", "a-profile", "b-profile", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()
    assert (sandbox / ".local/bin/demo-b").is_symlink()


def test_throttle_user_override_beats_plugin_budget(sandbox, monkeypatch):
    """env.toml [throttle] outranks the fetcher's own declaration.
    Self-relative: the budget-limited run (1/s across two fetches)
    sets this machine's baseline; the overridden run must beat it by
    the budget delay — wall-clock absolutes flake on slow runners."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    import time

    # baseline: the plugin's own 1/s budget delays the second fetch
    repo = make_workspace_repo(sandbox / "budgeted", PLUGIN_OUTPUTS)
    start = time.monotonic()
    out = grip("update", cwd=repo)
    budgeted = time.monotonic() - start
    assert out.returncode == 0, out.stderr

    # override: 100/s in env.toml outranks the declaration
    repo = make_workspace_repo(sandbox / "overridden", PLUGIN_OUTPUTS)
    with open(repo / "env.toml", "a") as f:
        f.write('\n[throttle]\n"demo.local" = "100/s"\n')
    start = time.monotonic()
    out = grip("update", cwd=repo)
    overridden = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert overridden < budgeted - 0.5, (
        f"override not applied: budgeted {budgeted:.2f}s, "
        f"overridden {overridden:.2f}s"
    )


@pytest.mark.parametrize("budget", ["NaN/s", "0.5/s"])
@pytest.mark.parametrize("override", [False, True])
def test_invalid_rate_cannot_panic_or_park_a_plugin(sandbox, budget, override):
    fixture = FETCH_FIXTURE if override else FETCH_FIXTURE.replace("1/s", budget)
    _seed_plugin_store(sandbox, "gripfetch-demo", fixture)
    repo = make_workspace_repo(sandbox / "myenv", {"a": PLUGIN_OUTPUTS["a"]})
    with (repo / "env.toml").open("a") as config:
        config.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
        if override:
            config.write(f'\n[throttle]\n"demo.local" = "{budget}"\n')
    out = run_grip([str(GRIP.resolve()), "update"], cwd=repo, capture_output=True, text=True, timeout=20,)
    assert out.returncode == 0, out.stdout + out.stderr
    applied = grip("apply", "a-profile", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    assert (sandbox / ".local/bin/demo-a").read_text() == "#!/bin/sh\necho demo\n"


def test_plugin_token_wait_is_bounded_and_failed_update_remains_recoverable(sandbox):
    invocations = sandbox / "fetch-invocations"
    fixture = FETCH_FIXTURE.replace("1/s", "1/hr").replace(
        'elif req["op"] == "fetch":\n',
        'elif req["op"] == "fetch":\n'
        f'    with open({str(invocations)!r}, "a") as record: record.write("fetch\\n")\n',
    )
    _seed_plugin_store(sandbox, "gripfetch-demo", fixture)
    repo = make_workspace_repo(sandbox / "myenv", PLUGIN_OUTPUTS)
    with (repo / "env.toml").open("a") as config:
        config.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    out = run_grip([str(GRIP.resolve()), "update"], cwd=repo, capture_output=True, text=True, timeout=20,)
    assert out.returncode == 1, out.stdout + out.stderr
    assert invocations.read_text() == "fetch\n"
    assert not (repo / "gripsack.lock").exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()
    with (repo / "env.toml").open("a") as config:
        config.write('\n[throttle]\n"demo.local" = "100/s"\n')
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    recovered = grip("apply", "a-profile", "b-profile", cwd=repo)
    assert recovered.returncode == 0, recovered.stdout + recovered.stderr
    for name in ("a", "b"):
        assert (sandbox / f".local/bin/demo-{name}").read_text() == "#!/bin/sh\necho demo\n"


def test_fetcher_package_ref_resolves_from_the_plugin_store(sandbox):
    """0012 move 2: [fetchers.x] package = "owner/repo@tag" — with the
    receipt already recording the tag, provisioning is a satisfied
    no-op (no network) and the fetch runs the store's binary, not PATH."""
    _seed_plugin_store(sandbox, "gripfetch-demo", FETCH_FIXTURE)
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { pluginFetch, pkg, provider, profile, file, artifactFile, identity, symlinkTo } from "@gripsack/core";

export default [
  pkg("a", {
    producer: provider(pluginFetch("demo")), commands: {},
    target: { os: "linux", arch: "x86_64" }, layout: { kind: "relocatable" },
  }),
  profile("a-profile", { files: [file({
    source: artifactFile("a", "bin/demo"), content: identity(),
    destination: symlinkTo("~/.local/bin/demo-a"),
  })] }),
];
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "a-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()


def test_plugin_cache_never_substitutes_a_same_tag_from_another_origin(sandbox, monkeypatch):
    invoked = sandbox / "wrong-origin-executed"
    fixture = FETCH_FIXTURE.replace(
        "req = json.loads(sys.stdin.readline())",
        f'open({str(invoked)!r}, "w").write("executed")\n'
        "req = json.loads(sys.stdin.readline())",
    )
    _seed_plugin_store(sandbox, "gripfetch-demo", fixture)
    receipt = sandbox / ".local/share/gripsack/plugins/receipts/gripfetch-demo.toml"
    original_receipt = receipt.read_bytes()
    repo = make_workspace_repo(sandbox / "myenv", {"a": PLUGIN_OUTPUTS["a"]})
    with (repo / "env.toml").open("a") as config:
        config.write('\n[fetchers.demo]\npackage = "different/gripfetch-demo@1.0"\n')
    # A bound, non-listening loopback port refuses provisioning offline. The
    # matching-origin control above must not need provisioning at all.
    with socket.socket() as refusal:
        refusal.bind(("127.0.0.1", 0))
        proxy = f"http://127.0.0.1:{refusal.getsockname()[1]}"
        for name in ("HTTPS_PROXY", "https_proxy"):
            monkeypatch.setenv(name, proxy)
        for name in ("NO_PROXY", "no_proxy"):
            monkeypatch.setenv(name, "")
        out = grip("update", cwd=repo)
    assert out.returncode == 1, out.stdout + out.stderr
    assert not invoked.exists()
    assert not (sandbox / ".local/bin/demo-a").exists()
    assert receipt.read_bytes() == original_receipt


def test_fetcher_path_registers_an_offline_executable(sandbox):
    """[fetchers.x] path registers an offline executable directly,
    without PATH lookup or online provisioning."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { pluginFetch, pkg, provider, profile, file, artifactFile, identity, symlinkTo } from "@gripsack/core";

export default [
  pkg("a", {
    producer: provider(pluginFetch("demo")), commands: {},
    target: { os: "linux", arch: "x86_64" }, layout: { kind: "relocatable" },
  }),
  profile("a-profile", { files: [file({
    source: artifactFile("a", "bin/demo"), content: identity(),
    destination: symlinkTo("~/.local/bin/demo-a"),
  })] }),
];
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write(f'\n[fetchers.demo]\npath = "{fetcher}"\n')
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "a-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()


def test_changed_fetch_spec_requires_explicit_update_instead_of_mirror_blame(sandbox):
    """Changed plugin arguments need explicit resolution, not mirror blame."""
    _seed_plugin_store(sandbox, "gripfetch-demo", FETCH_FIXTURE)
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { pluginFetch, pkg, provider, profile, file, artifactFile, identity, symlinkTo } from "@gripsack/core";

export default [
  pkg("a", {
    producer: provider(pluginFetch("demo")), commands: {},
    target: { os: "linux", arch: "x86_64" }, layout: { kind: "relocatable" },
  }),
  profile("a-profile", { files: [file({
    source: artifactFile("a", "bin/demo"), content: identity(),
    destination: symlinkTo("~/.local/bin/demo-a"),
  })] }),
];
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "a-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    # A frozen pin cannot silently follow a new plugin declaration.
    (sandbox / "myenv" / "outputs" / "hello.ts").write_text(
        """
import { pluginFetch, pkg, provider, profile, file, artifactFile, identity, symlinkTo } from "@gripsack/core";

export default [
  pkg("a", {
    producer: provider(pluginFetch("demo", { package: "hello", version: "1.0" })), commands: {},
    target: { os: "linux", arch: "x86_64" }, layout: { kind: "relocatable" },
  }),
  profile("a-profile", { files: [file({
    source: artifactFile("a", "bin/demo"), content: identity(),
    destination: symlinkTo("~/.local/bin/demo-a"),
  })] }),
];
"""
    )
    frozen = grip("apply", "a-profile", cwd=repo)
    assert frozen.returncode != 0, frozen.stdout + frozen.stderr
    assert "mirror" not in frozen.stdout + frozen.stderr
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "a-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "mirror" not in out.stderr


def test_mirror_swap_proves_then_dedups(sandbox):
    """An explicit mirror update proves byte identity before reusing the
    source tree; package receipts may change with the source declaration."""
    payload_a = make_tarball(sandbox / "a.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    payload_b = make_tarball(sandbox / "b.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    assert payload_a != payload_b
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, pkg, provider, profile, file, artifactFile, identity, symlinkTo }} from "@gripsack/core";

export default [
  pkg("hello", {{
    producer: provider(fileFetch("{payload_a}")), commands: {{}},
    target: {{ os: "linux", arch: "x86_64" }}, layout: {{ kind: "relocatable" }},
  }}),
  profile("hello-profile", {{ files: [file({{
    source: artifactFile("hello", "bin/hello"), content: identity(),
    destination: symlinkTo("~/.local/bin/hello"),
  }})] }}),
];
""",
    )
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "hello-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    before = (sandbox / ".local/bin/hello").read_bytes()
    pin_before = pins(repo)["hello"]
    assert pin_before["source"]["fetch"]["path"] == str(payload_a)
    store = sandbox / ".local/share/gripsack/store"
    sources_before = sorted(store.glob("*-workspace-source"))
    assert len(sources_before) == 1
    assert (sources_before[0] / "bin/hello").read_bytes() == before

    # the mirror swap: different URL, identical bytes
    output_ts = repo / "outputs" / "hello.ts"
    output_ts.write_text(output_ts.read_text().replace(str(payload_a), str(payload_b)))
    payload_a.unlink()
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    pin_after = pins(repo)["hello"]
    assert pin_after["source"]["fetch"]["path"] == str(payload_b)
    assert pin_after["resolved"]["tree256"] == pin_before["resolved"]["tree256"]
    out = grip("apply", "hello-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert sorted(store.glob("*-workspace-source")) == sources_before
    assert (sandbox / ".local/bin/hello").is_symlink()
    assert (sandbox / ".local/bin/hello").read_bytes() == before


def test_install_mapping_edit_does_not_refetch(sandbox):
    """0014 §3: install destinations are deploy concerns, not content —
    editing the mapping keeps the content path and skips the fetch."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, pkg, provider, profile, file, artifactFile, identity, symlinkTo }} from "@gripsack/core";

export default [
  pkg("hello", {{
    producer: provider(fileFetch("{payload}")), commands: {{}},
    target: {{ os: "linux", arch: "x86_64" }}, layout: {{ kind: "relocatable" }},
  }}),
  profile("hello-profile", {{ files: [file({{
    source: artifactFile("hello", "bin/hello"), content: identity(),
    destination: symlinkTo("~/.local/bin/hello"),
  }})] }}),
];
""",
    )
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "hello-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    before = (sandbox / ".local/bin/hello").resolve()
    store = sandbox / ".local/share/gripsack/store"
    stored_before = sorted(path.name for path in store.iterdir())

    output_ts = repo / "outputs" / "hello.ts"
    output_ts.write_text(
        output_ts.read_text().replace("~/.local/bin/hello", "~/.local/bin/hello2")
    )
    # Frozen acquisition must reuse the retained tree, not reopen the source.
    payload.unlink()
    out = grip("apply", "hello-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert set(stored_before).issubset(path.name for path in store.iterdir())
    assert "fetched" not in out.stdout
    assert (sandbox / ".local/bin/hello2").is_symlink()
    assert (sandbox / ".local/bin/hello2").resolve() == before


def test_fetch_placeholders_expand_from_host_facts(sandbox):
    """0016 §D1: {system}/{target}/{arch}/{arch.go}/{os} in a fetch URL
    expand from the machine's facts — one spec serves every platform."""
    import hashlib
    import threading
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    payload = make_tarball(sandbox / "pkg.tar.gz", {"bin/demo": b"#!/bin/sh\necho demo\n"})
    blob = payload.read_bytes()
    sha = hashlib.sha256(blob).hexdigest()

    # {system} on this host, per 0016 §D1's table
    machine = {"x86_64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}[os.uname().machine]
    system = f"{machine}-{'darwin' if sys.platform == 'darwin' else 'linux'}"

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == f"/pkg-{system}.tar.gz":
                self.send_response(200)
                self.send_header("Content-Length", str(len(blob)))
                self.end_headers()
                self.wfile.write(blob)
            else:
                self.send_response(404)
                self.end_headers()

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    port = server.server_address[1]

    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ tarball, pkg, provider, profile, file, artifactFile, identity, symlinkTo }} from "@gripsack/core";

export default [
  pkg("demo", {{
    producer: provider(tarball("http://127.0.0.1:{port}/pkg-{{system}}.tar.gz")), commands: {{}},
    target: {{ os: "linux", arch: "x86_64" }}, layout: {{ kind: "relocatable" }},
  }}),
  profile("demo-profile", {{ files: [file({{
    source: artifactFile("demo", "bin/demo"), content: identity(),
    destination: symlinkTo("~/.local/bin/demo"),
  }})] }}),
];
""",
    )
    try:
        updated = grip("update", cwd=repo)
        assert updated.returncode == 0, updated.stdout + updated.stderr
        out = grip("apply", "demo-profile", cwd=repo)
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
    # the server only answers the EXPANDED name — a 200 proves the
    # placeholder expanded from host facts
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo").is_symlink()
    # the lock records the spec verbatim and pins the CONTENT (hash);
    # Platform resolutions keep the declaration separate from acquired identity.
    lock = (repo / "gripsack.lock").read_text()
    assert sha in lock


def test_git_fetch_floats_and_the_lock_pins_head(sandbox):
    """0016 §D2: git(url) without a rev floats to the remote's HEAD —
    pinned into the lockfile by explicit update; a new upstream commit does
    NOT move an apply; `grip update` moves it deliberately."""
    git_env = {
        "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
        "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t",
    }
    remote = sandbox / "remote"
    remote.mkdir()
    for args in (["init", "-q"], ["add", "-A"]):
        subprocess.run(["git", *args], cwd=remote, env=git_env, check=True)
    (remote / "file.txt").write_text("v1\n")
    subprocess.run(["git", "add", "-A"], cwd=remote, env=git_env, check=True)
    subprocess.run(["git", "commit", "-qm", "v1"], cwd=remote, env=git_env, check=True)

    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ git, pkg, provider, profile, file, artifactFile, identity, symlinkTo }} from "@gripsack/core";

export default [
  pkg("tool", {{
    producer: provider(git("{remote}")), commands: {{}},
    target: {{ os: "linux", arch: "x86_64" }}, layout: {{ kind: "relocatable" }},
  }}),
  profile("tool-profile", {{ files: [file({{
    source: artifactFile("tool", "file.txt"), content: identity(),
    destination: symlinkTo("~/.local/share/tool/file.txt"),
  }})] }}),
];
""",
    )
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    out = grip("apply", "tool-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".local/share/tool/file.txt"
    assert deployed.read_text() == "v1\n"

    # upstream moves; an apply must NOT follow (the lock pins HEAD)
    (remote / "file.txt").write_text("v2\n")
    subprocess.run(["git", "commit", "-qam", "v2"], cwd=remote, env=git_env, check=True)
    out = grip("apply", "tool-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert deployed.read_text() == "v1\n"

    # update re-resolves HEAD and moves the pin
    out = grip("update", "tool", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", "tool-profile", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert deployed.read_text() == "v2\n"
