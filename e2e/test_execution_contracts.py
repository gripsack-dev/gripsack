"""Step ordering, scheduling, recipe outputs and verification receipts."""

from conftest import grip, make_env_repo, make_tarball


def test_explicit_steps_module_is_satisfied_on_reapply(sandbox):
    """Class/explicit-steps modules keep fetch specs in steps, not
    module.fetch — their store path must still be stable (canary-caught)."""
    payload = sandbox / "hello.tar.gz"
    make_tarball(payload, {"bin/x": b"#!/bin/sh\necho x\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fetchStep, module, shellStep, tarball }} from "@gripsack/core";

export default module("stepped", {{
  steps: [
    fetchStep(tarball("file://{payload}")),
    shellStep("true", "noop", {{ needs: ["fetch"] }}),
  ],
}});
""",
    )
    first = grip("apply", "--host", "testhost", cwd=repo)
    assert first.returncode == 0, first.stderr
    second = grip("apply", "--host", "testhost", cwd=repo)
    assert second.returncode == 0, second.stderr
    assert "already satisfied" in second.stdout


def test_step_needs_order_execution(sandbox):
    """0033 R4: a consumer declared BEFORE its producer still runs
    after it — `needs` orders execution, not just validation."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module } from "@gripsack/core";

export default module("demo", {
  steps: [
    {
      id: "consume",
      action: { kind: "custom_shell", script: "cat $HOME/marker >/dev/null" },
      needs: ["produce"],
      phase: "custom",
    },
    {
      id: "produce",
      action: { kind: "custom_shell", script: "echo made > $HOME/marker" },
      phase: "custom",
    },
  ],
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / "marker").read_text() == "made\n"


def test_a_dep_pin_update_rebuilds_the_consumer(sandbox):
    """0035 F4: the consumer's build key includes the dependency's
    RESOLVED pin — updating the producer's payload rebuilds the
    consumer, never serves a stale cache hit."""
    compiler = sandbox / "compiler.tar.gz"
    make_tarball(compiler, {"bin/cc": b"#!/bin/sh\necho cc-v1\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "compiler": f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("compiler", {{
  fetch: fileFetch("{compiler}"),
  install: {{ "bin/cc": symlink("~/.local/bin/cc") }},
}});
""",
            "consumer": """
import { dep, installStep, module, shellStep, symlink } from "@gripsack/core";

export default module("consumer", {
  depends: [dep("compiler")],
  steps: [
    shellStep("mkdir -p out && cp $HOME/.local/bin/cc out/built", "build"),
    installStep({ "out/built": symlink("~/.local/bin/built") }, "install", { needs: ["build"] }),
  ],
});
""",
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    built = sandbox / ".local/bin/built"
    assert built.read_text() == "#!/bin/sh\necho cc-v1\n"

    make_tarball(compiler, {"bin/cc": b"#!/bin/sh\necho cc-v2\n"})
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bumped" in out.stdout
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert built.read_text() == "#!/bin/sh\necho cc-v2\n", (
        "the consumer rebuilt against the new compiler"
    )


def test_a_failed_verifier_fails_every_retry(sandbox):
    """0035 F2: presence in the store is not a verification receipt —
    a permanently failing check fails EVERY apply, and a warm store
    never skips the deployment gate."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": """
import { module, trackedCopy, verifyShell } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") },
  verify: verifyShell("exit 1"),
});
"""
        },
    )
    for attempt in ["first", "second"]:
        out = grip("apply", "--host", "testhost", cwd=repo)
        assert out.returncode != 0, f"{attempt} apply must fail"
        assert not (sandbox / ".config/demo/a.conf").exists(), (
            f"{attempt} apply compensated the destination"
        )
    assert not (sandbox / ".local/share/gripsack/current").exists(), (
        "no generation ever activated"
    )


def test_a_fixed_verifier_verifies_once_then_receipt_holds(sandbox):
    """0035 F2, the other half: a passing check runs, the receipt rides
    the manifest, and the warm satisfied apply does not re-run it."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    marker = sandbox / "verify-ran"
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": f"""
import {{ module, trackedCopy, verifyShell }} from "@gripsack/core";

export default module("demo", {{
  config: {{ "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") }},
  verify: verifyShell("echo ran >> {marker}"),
}});
"""
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert marker.read_text().count("ran") == 1
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "satisfied" in out.stdout
    assert marker.read_text().count("ran") == 1, "the receipt holds on a satisfied apply"


def test_a_typo_field_is_rejected_never_pruned(sandbox):
    """0035 F3: `confg:` must fail eval with a suggestion — a typo is
    never a removal instruction."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") },
});
"""
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".config/demo/a.conf"
    assert dest.exists()

    (repo / "modules" / "demo.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  confg: { "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") },
});
"""
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "a typo'd field must fail check"
    assert "confg" in out.stderr and "config" in out.stderr, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert dest.exists(), "the typo must never prune"


def test_jobs_one_forces_serial_execution(sandbox):
    """--jobs bounds the scheduler (N3): the 2x2s parallel proof
    inverted — with --jobs 1 it must take serial time."""
    import time

    repo = make_env_repo(
        sandbox / "myenv",
        {
            name: f"""
import {{ module }} from "@gripsack/core";

export default module("{name}", {{
  build: {{ kind: "custom_shell", script: "sleep 2" }},
}});
"""
            for name in ("slow-a", "slow-b")
        },
    )
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", "--jobs", "1", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed >= 3.5, f"--jobs 1 not respected: {elapsed:.1f}s"


def test_independent_modules_run_in_parallel(sandbox):
    """Two independent 2s builds overlap (0007 §5 — the ready-queue
    scheduler runs N = cores). Self-relative: measure a --jobs 1
    baseline in this same sandbox, then assert the default-jobs run
    beats it by the overlap — wall-clock absolutes flake on slow
    runners (the macOS round taught this)."""
    import time

    modules = {
        name: f"""
import {{ module }} from "@gripsack/core";

export default module("{name}", {{
  build: {{ kind: "custom_shell", script: "sleep 2" }},
}});
"""
        for name in ("slow-a", "slow-b")
    }

    repo = make_env_repo(sandbox / "serial", modules)
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", "--jobs", "1", cwd=repo)
    serial = time.monotonic() - start
    assert out.returncode == 0, out.stderr

    repo = make_env_repo(sandbox / "parallel", modules)
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    parallel = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    # two 2s builds: serial ~4s+overhead, parallel ~2s+overhead — the
    # speedup must recover at least half the second sleep
    assert parallel < serial - 0.8, (
        f"no overlap detected: serial {serial:.1f}s, parallel {parallel:.1f}s"
    )


def test_jobs_zero_is_rejected(sandbox):
    """Finding B: --jobs 0 / GRIPSACK_JOBS=0 must fail loudly, never
    silently unmanage the environment."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("a", {
  config: { "configs/a/a": trackedCopy("~/.out/a") },
});
""",
    )
    (repo / "configs" / "a").mkdir(parents=True)
    (repo / "configs" / "a" / "a").write_text("a\n")
    out = grip("apply", "--host", "testhost", "--jobs", "0", cwd=repo)
    assert out.returncode != 0
    assert "--jobs 0" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_run_steps_execute_with_declared_outputs(sandbox):
    """run steps (0007 §3 rung 2): structured argv, no shell — declared
    outputs are the contract and a missing one is a step error."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fetchStep, installStep, module, runStep, symlink, tarball }} from "@gripsack/core";

export default module("hello", {{
  steps: [
    fetchStep(tarball("file://{payload}")),
    runStep(["cp", "bin/hello", "bin/hello-copy"], "copy", {{ outputs: ["bin/hello-copy"] }}),
    installStep({{ "bin/hello-copy": symlink("~/.local/bin/hello-copy") }}),
  ],
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello-copy").is_symlink()
