"""Read-only preview and agreement with executed destination operations."""

from conftest import grip, make_env_repo, make_tarball, refresh_host


def test_plan_renders_the_same_ops_apply_executes(sandbox):
    """0034: the preview IS the operation list — plan/apply agreement
    by construction. New, satisfied, update, drift-kept, and prune all
    render what apply then does."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("v1\n")
    (confdir / "b.toml").write_text("b\n")
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: {
    "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml"),
    "configs/demo/b.toml": trackedCopy("~/.config/demo/b.toml"),
  },
});
"""
        },
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "+ configs/demo/a.toml → ~/.config/demo/a.toml (new)" in out.stdout, out.stdout
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config/demo/a.toml").read_text() == "v1\n"

    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "= ~/.config/demo/a.toml (satisfied)" in out.stdout, out.stdout

    # drift: the preview says kept, not update (the old preview lied)
    (sandbox / ".config/demo/a.toml").write_text("mine\n")
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "~/.config/demo/a.toml drifted — kept" in out.stdout, out.stdout
    assert "(update)" not in out.stdout
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert "drifted" in out.stdout
    assert (sandbox / ".config/demo/a.toml").read_text() == "mine\n"

    # update + prune in one preview
    (repo / "configs" / "demo" / "a.toml").write_text("v2\n")
    (repo / "modules" / "demo.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
"""
    )
    refresh_host(repo)
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "- ~/.config/demo/b.toml (prune)" in out.stdout, out.stdout
    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not (sandbox / ".config/demo/b.toml").exists()
    assert (sandbox / ".config/demo/a.toml").read_text() == "v2\n"


def test_plan_marks_opaque_run_steps(sandbox):
    """0033 R5 + 0034: a run-step module never reads as a no-op."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module } from "@gripsack/core";

export default module("demo", {
  steps: [
    {
      id: "build",
      action: { kind: "run", argv: ["true"], outputs: [] },
      phase: "custom",
    },
  ],
});
""",
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "opaque effects" in out.stdout, out.stdout
    assert "nothing would change" not in out.stdout


def test_plan_creates_nothing_and_reads_state_correctly(sandbox):
    """0035 F7: plan is read-only (no parent dirs appear), a satisfied
    owned link previews as satisfied, and a deferred fetched
    destination is never shown as a prune."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hi\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "hello": f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
            "demo": """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") },
});
""",
        },
    )
    (repo / "configs" / "demo").mkdir(parents=True)
    (repo / "configs" / "demo" / "a.conf").write_text("a\n")
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not (sandbox / ".config").exists(), "plan created a directory"

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello").is_symlink()
    assert (sandbox / ".config/demo/a.conf").exists()

    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "= ~/.local/bin/hello (satisfied)" in out.stdout, out.stdout
    assert "+ " not in out.stdout, "a satisfied link must not preview as new"
    assert "(prune)" not in out.stdout, "a deferred dest is still declared"
