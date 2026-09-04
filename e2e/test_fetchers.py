"""Fetcher e2e: tarball/git/plugin fetch specs, throttle, the managed
plugin store, and store dedup identities — split from test_flow.py;
fixture repos come from conftest."""



import os
import sys
import subprocess

from conftest import (
    _seed_plugin_store,
    grip,
    make_env_repo,
    make_tarball,
    only_store_path,
)



def test_deferred_identity_is_stable_across_applies(sandbox):
    """Finding C: fetch kinds whose payload hash isn't knowable up
    front (git/pixi/plugin) must produce ONE store path and satisfy
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

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ git, module, symlink }} from "@gripsack/core";

export default module("tool", {{
  fetch: git("file://{remote}", "{rev}"),
  install: {{ "bin.txt": symlink("~/.local/bin/tool") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "applied" in out.stdout
    store = sandbox / ".local/share/gripsack/store"
    paths_after_first = sorted(p.name for p in store.iterdir())
    assert len(paths_after_first) == 1

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "already satisfied" in out.stdout, out.stdout
    assert sorted(p.name for p in store.iterdir()) == paths_after_first


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


PLUGIN_MODULES = {
    name: """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("%s", {
  fetch: pluginFetch("demo"),
  install: { "bin/demo": symlink("~/.local/bin/demo-%s") },
});
"""
    % (name, suffix)
    for name, suffix in (("a", "a"), ("b", "b"))
}


def test_throttle_token_bucket_serializes_plugin_fetches(sandbox, monkeypatch):
    """[throttle] (0002): a fetcher declares its rate budget via the
    capabilities op; the core's token bucket enforces it across
    concurrent modules — two fetches at 1/s take >= ~1s wall."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    repo = make_env_repo(sandbox / "myenv", PLUGIN_MODULES)
    import time

    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed >= 0.9, f"two 1/s fetches finished in {elapsed:.2f}s — bucket not enforced"
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
    repo = make_env_repo(sandbox / "budgeted", PLUGIN_MODULES)
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    budgeted = time.monotonic() - start
    assert out.returncode == 0, out.stderr

    # override: 100/s in env.toml outranks the declaration
    repo = make_env_repo(sandbox / "overridden", PLUGIN_MODULES)
    with open(repo / "env.toml", "a") as f:
        f.write('\n[throttle]\n"demo.local" = "100/s"\n')
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    overridden = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert overridden < budgeted - 0.5, (
        f"override not applied: budgeted {budgeted:.2f}s, "
        f"overridden {overridden:.2f}s"
    )


def test_fetcher_package_ref_resolves_from_the_plugin_store(sandbox):
    """0012 move 2: [fetchers.x] package = "owner/repo@tag" — with the
    receipt already recording the tag, provisioning is a satisfied
    no-op (no network) and the fetch runs the store's binary, not PATH."""
    _seed_plugin_store(sandbox, "gripfetch-demo", FETCH_FIXTURE)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("a", {
  fetch: pluginFetch("demo"),
  install: { "bin/demo": symlink("~/.local/bin/demo-a") },
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()


def test_fetcher_path_registers_an_offline_executable(sandbox):
    """[fetchers.x] path = ... registers the executable directly —
    the offline/air-gapped route, symmetric with [linters.x] path
    (enterprise review). No PATH lookup, no provisioning."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("a", {
  fetch: pluginFetch("demo"),
  install: { "bin/demo": symlink("~/.local/bin/demo-a") },
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write(f'\n[fetchers.demo]\npath = "{fetcher}"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()


def test_changed_fetch_spec_re_resolves_instead_of_mirror_blame(sandbox):
    """A fetch-spec edit must not fail as 'the mirror changed' — the
    args are the declaration and the pin follows them (enterprise
    review's stale-plugin-lock papercut)."""
    _seed_plugin_store(sandbox, "gripfetch-demo", FETCH_FIXTURE)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("a", {
  fetch: pluginFetch("demo"),
  install: { "bin/demo": symlink("~/.local/bin/demo-a") },
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    # same module, now pinned — the lock entry for the OLD args must
    # not compare against the NEW spec
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("a", {
  fetch: pluginFetch("demo", { package: "hello", version: "1.0" }),
  install: { "bin/demo": symlink("~/.local/bin/demo-a") },
});
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "mirror" not in out.stderr


def test_mirror_swap_proves_then_dedups(sandbox):
    """0014 §3: the recipe left the store path — a changed fetch spec
    must re-fetch once to PROVE byte identity, then dedup to the same
    content path: no second store entry, no redeploy."""
    payload_a = make_tarball(sandbox / "a.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    payload_b = make_tarball(sandbox / "b.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    assert payload_a != payload_b
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload_a}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    before = only_store_path(sandbox)

    # the mirror swap: different URL, identical bytes
    module_ts = repo / "modules" / "hello.ts"
    module_ts.write_text(module_ts.read_text().replace(str(payload_a), str(payload_b)))
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert only_store_path(sandbox) == before
    # one fetch to prove identity, then nothing moved: no redeploy, no
    # new generation
    assert "fetched" in out.stdout
    assert "linked" not in out.stdout
    assert "already satisfied" in out.stdout


def test_install_mapping_edit_does_not_refetch(sandbox):
    """0014 §3: install destinations are deploy concerns, not content —
    editing the mapping keeps the content path and skips the fetch."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    before = only_store_path(sandbox)

    module_ts = repo / "modules" / "hello.ts"
    module_ts.write_text(
        module_ts.read_text().replace("~/.local/bin/hello", "~/.local/bin/hello2")
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert only_store_path(sandbox) == before
    assert "content already in store" in out.stdout
    assert "fetched" not in out.stdout
    assert (sandbox / ".local/bin/hello2").is_symlink()


def test_fetch_placeholders_expand_from_host_facts(sandbox):
    """0016 §D1: {system}/{target}/{arch}/{arch.go}/{os} in a fetch URL
    expand from the machine's facts — one spec serves every platform."""
    import hashlib
    import io
    import tarfile
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
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ module, symlink, tarball }} from "@gripsack/core";

export default module("demo", {{
  fetch: tarball("http://127.0.0.1:{port}/pkg-{{system}}.tar.gz"),
  install: {{ "bin/demo": symlink("~/.local/bin/demo") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    server.shutdown()
    # the server only answers the EXPANDED name — a 200 proves the
    # placeholder expanded from host facts
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo").is_symlink()
    # the lock records the spec verbatim and pins the CONTENT (hash);
    # per-host locks (0001 §5) expand per machine at fetch time
    lock = (repo / "locks/testhost.lock").read_text()
    assert sha in lock


def test_git_fetch_floats_and_the_lock_pins_head(sandbox):
    """0016 §D2: git(url) without a rev floats to the remote's HEAD —
    pinned into the lockfile at first apply; a new upstream commit does
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

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ git, module, symlink }} from "@gripsack/core";

export default module("tool", {{
  fetch: git("{remote}"),
  install: {{ "file.txt": symlink("~/.local/share/tool/file.txt") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".local/share/tool/file.txt"
    assert deployed.read_text() == "v1\n"

    # upstream moves; an apply must NOT follow (the lock pins HEAD)
    (remote / "file.txt").write_text("v2\n")
    subprocess.run(["git", "commit", "-qam", "v2"], cwd=remote, env=git_env, check=True)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert deployed.read_text() == "v1\n"

    # update re-resolves HEAD and moves the pin
    out = grip("update", "tool", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert deployed.read_text() == "v2\n"


def test_pinned_git_lock_survives_apply_and_update_says_pinned(sandbox):
    """A git rev IS the pin: apply records it as the lock's version
    (and keeps it across re-applies), and `grip update` says so."""
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
    subprocess.run(["git", "init", "--quiet"], cwd=remote, env=env, check=True)
    (remote / "bin.txt").write_text("payload\n")
    subprocess.run(["git", "add", "."], cwd=remote, env=env, check=True)
    subprocess.run(["git", "commit", "--quiet", "-m", "init"], cwd=remote, env=env, check=True)
    rev = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=remote, env=env, check=True, capture_output=True, text=True
    ).stdout.strip()

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ git, module, symlink }} from "@gripsack/core";

export default module("tpm", {{
  fetch: git("file://{remote}", "{rev}"),
  install: {{ "bin.txt": symlink("~/.local/bin/tpm") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    lock = repo / "locks" / "testhost.lock"
    assert f'"version": "{rev}"' in lock.read_text()
    # a warm re-apply must not drop the pin
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert f'"version": "{rev}"' in lock.read_text()

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "pinned by rev" in out.stdout
    assert "not supported" not in out.stdout


def test_update_resolves_a_fetch_step_module(sandbox):
    """F1 regression: a steps-style module with one fetch step pins
    exactly like the declarative style — update used to walk only
    module.fetch, so converting a module to the class style silently
    dropped it out of the lockfile resolver ("nothing to resolve yet")
    while check/plan stayed quiet."""
    payload = make_tarball(
        sandbox / "p.tar.gz", {"bin/tool": b"#!/bin/sh\necho tool\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("stepped", {{
  steps: [{{
    id: "fetch",
    action: {{ kind: "fetch", fetch: fileFetch("{payload}") }},
  }}],
}});
""",
    )
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "nothing to resolve" not in out.stdout, (
        "a fetch step must reach the lockfile resolver"
    )
    lock = (repo / "locks" / "testhost.lock").read_text()
    assert "stepped" in lock, out.stdout
