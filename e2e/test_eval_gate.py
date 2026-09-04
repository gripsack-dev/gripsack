"""Trust gate and eval e2e: trust add/remove, the probe fixpoint,
[eval] env, check/plan diagnostics, host resolution, init/doctor,
self-update — split from test_flow.py; fixture repos come from conftest."""



import os
import shutil
import subprocess

from conftest import (
    GRIP,
    grip,
    make_env_repo,
)



HELLO_MODULE = """
import { module, trackedCopy } from "@gripsack/core";

export default module("hello", {
  config: { "configs/demo/a": trackedCopy("~/.config/demo/a") },
});
"""


def test_binary_exists_and_runs():
    out = grip("--version")
    assert out.returncode == 0
    assert "grip" in out.stdout


def test_doctor_reports_environment(sandbox):
    out = grip("doctor")
    # exit code depends on the sandbox's runtime being acceptable to
    # doctor; the report itself is the contract.
    assert "deno:" in out.stdout
    assert "frontend:" in out.stdout
    assert "home:" in out.stdout
    assert str(sandbox) in out.stdout


def test_untrusted_repo_fails_closed_without_tty(sandbox, monkeypatch):
    """The trust gate (0013 D7): first eval of an untrusted repo, no TTY
    to prompt on → hard error with the escape hatch, never a silent
    eval of unreviewed repo code."""
    monkeypatch.delenv("GRIPSACK_TRUST_ALL", raising=False)
    repo = make_env_repo(sandbox / "myenv", HELLO_MODULE)
    (repo / "configs" / "demo").mkdir(parents=True)
    (repo / "configs" / "demo" / "a").write_text("a\n")
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "trust" in out.stderr.lower()
    assert "grip trust add" in out.stderr
    # nothing ran: eval never happened
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_trust_add_records_and_unblocks_eval(sandbox, monkeypatch):
    """`grip trust add <path>` records the canonical path; eval then
    proceeds without the CI bypass; remove re-arms the gate."""
    monkeypatch.delenv("GRIPSACK_TRUST_ALL", raising=False)
    repo = make_env_repo(sandbox / "myenv", HELLO_MODULE)
    (repo / "configs" / "demo").mkdir(parents=True)
    (repo / "configs" / "demo" / "a").write_text("a\n")

    out = grip("trust", "add", str(repo))
    assert out.returncode == 0, out.stderr
    trust = sandbox / ".local/share/gripsack" / "trust.toml"
    assert "[[repos]]" in trust.read_text()
    assert str(repo) in trust.read_text()

    listing = grip("trust", "list")
    assert listing.returncode == 0, listing.stderr
    assert str(repo) in listing.stdout

    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    out = grip("trust", "remove", str(repo))
    assert out.returncode == 0, out.stderr
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "trust" in out.stderr.lower()


def test_eval_env_reaches_build_steps(sandbox):
    """[eval] env (build-time exported env): env.toml declares it,
    a build step's shell inherits it."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, shellStep } from "@gripsack/core";

export default module("probe", {
  steps: [shellStep('test "$MY_CERT_PATH" = "/etc/ssl/company.pem"', "probe")],
});
""",
    )
    (repo / "env.toml").write_text(
        '[env]\nname = "fixture"\n\n[eval]\nenv = { MY_CERT_PATH = "/etc/ssl/company.pem" }\n'
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_probe_fixpoint_converges_in_two_rounds(sandbox):
    """The probe loop (0013 D6) is bounded demand-driven re-eval: a
    healthy frontend requests its probes in round 1, sees them bound
    in round 2, and requests nothing new — exactly 2 frontend runs.
    More means probe caching broke (every eval paying 4 rounds would
    be a silent 2x slowdown); the round cap is for non-convergence,
    not the happy path."""
    import json

    repo = make_env_repo(
        sandbox / "myenv",
        {
            "hosted": """
import { module } from "@gripsack/core";

export default module("hosted", { install: [] });
"""
        },
    )
    # a host that actually calls ctx.probe
    (repo / "hosts" / "testhost.ts").write_text(
        'import { defineEnv } from "@gripsack/core";\n'
        'import hosted from "../modules/hosted.ts";\n\n'
        "export default defineEnv((ctx) => ({\n"
        '  tags: ["probe"],\n'
        "  modules: [ctx.probe.executable(\"sh\") && hosted],\n"
        "}));\n"
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    runs = sandbox / ".local/share/gripsack/runs"
    latest = (runs / "latest").resolve()
    rounds = []
    for line in latest.read_text().splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("message") == "frontend eval" or "frontend eval" in str(
            event.get("fields", {}).get("message", "")
 ):
            fields = event.get("fields", event)
            rounds.append(fields.get("round"))
    assert rounds == [1, 2], f"expected exactly two eval rounds, got {rounds}"


def test_check_fails_statically_on_missing_config_source(sandbox):
    """E110 (review finding E2): a fetch-less module's source must be
    a repo file — missing is a check-time error, not mid-deploy."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("aaa", {
  config: { "configs/aaa/MISSING.conf": trackedCopy("~/.out/a.conf") },
});
""",
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E110" in out.stderr
    assert "MISSING.conf" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_init_scaffolds_a_working_env_repo(sandbox):
    """grip init: embedded template (offline, version-matched), never
    clobbers an existing env repo."""
    repo = sandbox / "fresh"
    repo.mkdir()
    out = grip("init", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / "env.toml").exists()
    assert (repo / "modules" / "hello.ts").exists()
    assert (repo / "modules" / "examples.ts").exists()
    assert (repo / "configs" / "hello" / "hello.toml").exists()
    assert (repo / "package.json").exists()
    assert '"@gripsack/core"' in (repo / "package.json").read_text()
    assert (repo / "tsconfig.json").exists()
    assert "node_modules/" in (repo / ".gitignore").read_text()
    hosts = list((repo / "hosts").glob("*.ts"))
    assert len(hosts) == 1
    assert (repo / ".git").is_dir()

    # the scaffold is a working repo: check and apply succeed offline
    out = grip("check", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config" / "hello" / "hello.toml").is_symlink()

    # init never clobbers an existing env repo
    out = grip("init", cwd=repo)
    assert out.returncode != 0
    assert "already looks like an env repo" in out.stderr


def test_unmatched_host_errors_instead_of_empty_tags(sandbox):
    """A hosts/ dir with no matching entrypoint must not silently
    yield empty tags — every gated module would drop and the run would
    report success (enterprise review finding)."""
    repo = make_env_repo(sandbox / "myenv", HELLO_MODULE)
    (sandbox / "myenv" / "configs" / "demo").mkdir(parents=True)
    (sandbox / "myenv" / "configs" / "demo" / "a").write_text("a\n")
    out = grip("check", "--host", "nosuchhost", cwd=repo)
    assert out.returncode != 0
    assert "nosuchhost" in out.stderr


def test_default_host_resolves_role_named_entrypoint(sandbox):
    """[env] default_host: containers with random hostnames still get a
    deterministic host entrypoint (enterprise review finding)."""
    repo = make_env_repo(sandbox / "myenv", HELLO_MODULE)
    (sandbox / "myenv" / "configs" / "demo").mkdir(parents=True)
    (sandbox / "myenv" / "configs" / "demo" / "a").write_text("a\n")
    (sandbox / "myenv" / "hosts" / "testhost.ts").unlink()
    (sandbox / "myenv" / "hosts" / "role.ts").write_text(
        'import { defineEnv } from "@gripsack/core";\n'
        "import hello from \"../modules/hello.ts\";\n"
        "\n"
        "export default defineEnv((ctx) => ({\n"
        '  tags: ["container"],\n'
        "  modules: [hello],\n"
        "}));\n"
    )
    env_toml = repo / "env.toml"
    env_toml.write_text(env_toml.read_text().replace(
        '[env]\nname = "fixture"', '[env]\nname = "fixture"\ndefault_host = "role"'))
    out = grip("check", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "container" in out.stdout


def test_plan_diffs_against_the_current_generation(sandbox):
    """plan shows what apply WOULD do: new/update/satisfied/prune, plus
    the take-over warning for foreign paths (0004 pass 5)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
""",
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "+ configs/demo/a.toml" in out.stdout

    grip("apply", "--host", "testhost", cwd=repo)
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "= ~/.config/demo/a.toml (satisfied)" in out.stdout

    (confdir / "a.toml").write_text("b\n")
    (confdir / "b.toml").write_text("b\n")
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: {
    "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml"),
    "configs/demo/b.toml": trackedCopy("~/.config/demo/b.toml"),
  },
});
"""
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "~ configs/demo/a.toml" in out.stdout
    assert "+ configs/demo/b.toml" in out.stdout

    # apply the two-entry module, then drop b → the next plan prunes it
    grip("apply", "--host", "testhost", cwd=repo)

    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
"""
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "- ~/.config/demo/b.toml (prune)" in out.stdout


def test_plan_compares_template_and_merge_in_deployed_terms(sandbox):
    """plan must answer "is anything drifting?" honestly: template and
    merge entries compare as their DEPLOYED form (rendered bytes,
    trimmed block), not the raw repo source — the raw source can never
    match by construction, and the permanent phantom (update) trained
    users to ignore plan's output (0.21.1 review round)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "app.toml").write_text('key = "{{ value }}"\n')
    (confdir / "block.sh").write_text("export DEMO=1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module, template } from "@gripsack/core";

export default module("demo", {
  config: {
    "configs/demo/app.toml": template("~/.config/demo/app.toml", { value: "rendered" }),
    "configs/demo/block.sh": merge("~/.bashrc"),
  },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert 'key = "rendered"' in (sandbox / ".config/demo/app.toml").read_text()

    # steady state: no phantom updates
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "(update)" not in out.stdout, out.stdout
    assert "= ~/.config/demo/app.toml (satisfied)" in out.stdout
    assert "= ~/.bashrc (satisfied)" in out.stdout

    # real drift still reports: edit the deployed merge block by hand
    bashrc = sandbox / ".bashrc"
    bashrc.write_text(bashrc.read_text().replace("DEMO=1", "DEMO=2"))
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "(update)" in out.stdout, out.stdout


def test_exec_failures_render_with_codes_and_spans(sandbox):
    """0004 §3 across the stack: a fetch failure at apply is a coded,
    span-labeled diagnostic pointing at the module — never a bare
    `error:` line. (The placeholder E114 path is unit-tested in
    gripsack-ir; this covers the apply renderer.)"""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tarball } from "@gripsack/core";

export default module("demo", {
  fetch: tarball("http://127.0.0.1:1/never-there.tar.gz"),
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E301" in out.stderr
    assert "modules/hello.ts" in out.stderr  # make_env_repo's default filename
    assert "raised here" in out.stderr


def test_third_party_npm_deps_resolve_in_module_code(sandbox):
    """Repo npm dependencies work in module code: the eval sandbox is
    read-only within the repo (node_modules included), and the pin map
    is applied via --import-map (not deno.json discovery) so BYONM
    still engages. Repos without package.json keep the embedded copy."""
    repo = make_env_repo(sandbox / "myenv", {})
    pkg = repo / "node_modules/tinypkg"
    pkg.mkdir(parents=True)
    (pkg / "package.json").write_text(
        '{"name": "tinypkg", "version": "1.0.0", "type": "module", "main": "index.js"}'
    )
    (pkg / "index.js").write_text('export const answer = 42;\n')
    (repo / "package.json").write_text(
        '{"name": "fixture", "private": true, "type": "module",'
        ' "dependencies": {"tinypkg": "1.0.0"}}'
    )
    (repo / "modules" / "usesdep.ts").write_text(
        'import { answer } from "tinypkg";\n'
        'import { module, tree } from "@gripsack/core";\n'
        'export default module("usesdep", {\n'
        '  config: tree("configs/demo", "~/.config/demo", answer === 42 ? "owned" : "merge"),\n'
        '});\n'
    )
    confdir = repo / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    # rewrite the host to include the new module
    host = repo / "hosts" / "testhost.ts"
    src = host.read_text().replace("modules: []", "modules: [usesdep]")
    src = src.replace(
        'import { defineEnv } from "@gripsack/core";',
        'import { defineEnv } from "@gripsack/core";\nimport usesdep from "../modules/usesdep.ts";',
    )
    host.write_text(src)
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_embedded_frontend_serves_without_node_modules(sandbox):
    """The frontend source ships in the binary (0013 D3): a repo with
    NO node_modules evals offline against the embedded copy materialized
    under $GRIPSACK_HOME/frontend/ts-<version>/. Only the runtime is
    provisioned — and the sandbox inherits deno from PATH, so nothing
    downloads (e2e is offline)."""
    repo = make_env_repo(
        sandbox / "env",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("c", {
  config: { "x.toml": trackedCopy("~/.config/x.toml") },
});
""",
    )
    (repo / "x.toml").write_text("embedded = true\n")
    assert not (repo / "node_modules").exists()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config/x.toml").read_text() == "embedded = true\n"

    # the embedded copy was materialized at the core's version…
    gs = sandbox / ".local/share/gripsack"
    versions = sorted((gs / "frontend").glob("ts-*"))
    assert versions, "embedded frontend not materialized"
    assert (versions[-1] / "src" / "cli.ts").exists()
    # …and no runtime was fetched when deno is external (PATH or
    # GRIPSACK_DENO — the gate image always provides it; the offline
    # proof only means anything there)
    deno_external = os.environ.get("GRIPSACK_DENO") or shutil.which("deno")
    tools = gs / "tools"
    if deno_external:
        assert not tools.exists() or not list(tools.iterdir()), \
            "deno was external but a tool was downloaded anyway"

    # re-apply is satisfied — the embedded path is stable, not a one-shot
    again = grip("apply", "--host", "testhost", cwd=repo)
    assert again.returncode == 0, again.stderr
    assert "already satisfied" in again.stdout


def _core_version() -> str:
    out = subprocess.run(
        [str(GRIP.resolve()), "--version"], capture_output=True, text=True
    )
    return out.stdout.strip().split()[-1]


def _fake_release_server(sandbox, latest: str):
    """A loopback GitHub releases API for self-update tests: one core-v
    release, the platform tarball (a fake `grip`), and its sha256
    sidecar — served for all four release triples so any host matches."""
    import hashlib
    import io
    import json
    import tarfile
    import threading
    from http.server import BaseHTTPRequestHandler, HTTPServer

    fake = b"#!/bin/sh\necho fake-grip-" + latest.encode() + b"\n"
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        # the real release layout nests: gripsack-<v>-<triple>/grip
        info = tarfile.TarInfo(f"gripsack-{latest}-x86_64-unknown-linux-musl/grip")
        info.size = len(fake)
        info.mode = 0o755
        tar.addfile(info, io.BytesIO(fake))
    tarball = buf.getvalue()
    sha = hashlib.sha256(tarball).hexdigest()

    triples = [
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    ]
    base = "http://127.0.0.1:PORT"
    assets = [
        {
            "name": f"gripsack-{latest}-{t}.tar.gz",
            "browser_download_url": f"{base}/dl/t.tar.gz",
            "url": f"{base}/api/t.tar.gz",
        }
        for t in triples
    ] + [
        {
            "name": f"gripsack-{latest}-{t}.tar.gz.sha256",
            "browser_download_url": f"{base}/dl/t.sha256",
            "url": f"{base}/api/t.sha256",
        }
        for t in triples
    ]
    listing = json.dumps([
        {"tag_name": "ts-v99.0.0", "assets": []},
        {"tag_name": f"core-v{latest}", "assets": assets},
    ]).encode()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path.startswith("/repos/"):
                body = listing
            elif self.path.endswith("t.tar.gz"):
                body = tarball
            elif self.path.endswith("t.sha256"):
                body = f"{sha}  gripsack.tar.gz\n".encode()
            else:
                self.send_response(404)
                self.end_headers()
                return
            self.send_response(200)
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    listing = listing.replace(b"PORT", str(server.server_port).encode())
    return server, f"http://127.0.0.1:{server.server_port}"


def test_self_update_check_and_swap(sandbox, monkeypatch):
    """self-update is the package manager dogfood: resolve the newest
    core-v release (ts-v tags don't fool it), verify the sha256 sidecar,
    stage, and atomically rename over the running binary."""
    import shutil

    server, api = _fake_release_server(sandbox, "9.9.9")
    monkeypatch.setenv("GRIPSACK_UPDATE_API", api)
    fake_bin = sandbox / "bin"
    fake_bin.mkdir()
    exe = fake_bin / "grip"
    shutil.copy(GRIP.resolve(), exe)
    exe.chmod(0o755)

    def self_update(*args):
        return subprocess.run(
            [str(exe), "self-update", *args], capture_output=True, text=True, timeout=60
        )

    out = self_update("--check")
    assert out.returncode == 0, out.stderr
    assert "9.9.9" in out.stdout and "available" in out.stdout
    before = exe.read_bytes()
    out = self_update()
    assert out.returncode == 0, out.stderr
    assert "updated" in out.stdout and "9.9.9" in out.stdout
    assert exe.read_bytes() != before
    swapped = subprocess.run([str(exe)], capture_output=True, text=True)
    assert swapped.stdout.strip() == "fake-grip-9.9.9"
    server.shutdown()


def test_self_update_already_current(sandbox, monkeypatch):
    server, api = _fake_release_server(sandbox, _core_version())
    monkeypatch.setenv("GRIPSACK_UPDATE_API", api)
    out = subprocess.run(
        [str(GRIP.resolve()), "self-update", "--check"],
        capture_output=True, text=True, timeout=60,
    )
    assert out.returncode == 0, out.stderr
    assert "is current" in out.stdout
    server.shutdown()


def test_update_reports_modules_outside_the_hosts_graph(sandbox):
    """F3 regression: a name passed to `grip update` that this host's
    graph does not declare (probe-gated or typo) used to vanish
    silently — eight asked, seven answered, exit 0."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module } from "@gripsack/core";

export default module("present", { install: [] });
""",
    )
    out = grip("update", "present", "ghost", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    lines = out.stdout
    assert "ghost" in lines, out.stdout
    assert "not in this host's graph" in lines, out.stdout


def test_apply_refuses_unknown_module_names(sandbox):
    """The same silence on apply would be worse — an apply that
    'succeeded' while ignoring part of the request lies."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module } from "@gripsack/core";

export default module("present", { install: [] });
""",
    )
    out = grip("apply", "ghost", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "not in this host's graph" in out.stderr, out.stderr
    assert "ghost" in out.stderr


def test_doctor_warns_on_a_stale_core_pin(sandbox):
    """The repo's package.json @gripsack/core pin is what the editor
    and tsc typecheck against (the deliberate-pin rule); a stale one
    silently accepts authoring styles the embedded frontend removed.
    Doctor compares major.minor and names the upgrade command."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module } from "@gripsack/core";

export default module("a", { install: [] });
""",
    )
    (repo / "package.json").write_text(
        '{"devDependencies": {"@gripsack/core": "^0.17.5"}}\n'
    )
    out = grip("doctor", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "repo pin:" in out.stdout, out.stdout
    # a stale pin is a warning, not a pass — the 0.21.0 line rendered
    # as a green "ok" (a no-op marker replace) and advised an npm
    # version that did not exist yet
    pin_line = next(l for l in out.stdout.splitlines() if "repo pin:" in l)
    assert pin_line.startswith("warn"), pin_line
    assert "@gripsack/core ^0.17.5" in out.stdout
    assert "npm i -D @gripsack/core@" in out.stdout
    # the advice pins the minor LINE (^M.m.0): ^0.21.1 cannot resolve
    # when npm's latest is 0.21.0, and the frontend doesn't republish
    # on every core patch
    assert "@^{}.{}.0)".format(*out.stdout.split("frontend is ")[1].split(" —")[0].split(".")[:2]) in out.stdout, out.stdout
