"""Apply lifecycle: production, publication, source changes and pruning."""

import os
import shutil
import subprocess
from conftest import grip, make_env_repo, make_tarball


def test_apply_creates_generation_and_symlinks(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
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
    home = sandbox / ".local/share/gripsack"
    assert (home / "generations/1").is_dir()
    assert (home / "current").is_symlink()
    assert (sandbox / ".local/bin/hello").is_symlink()


def test_fetchless_build_outputs_reach_the_store(sandbox):
    """A module whose content is a build step (no fetch) must keep
    its outputs: publish used to hand-roll a fresh staging dir and
    wipe what the step had just produced — apply "succeeded" with an
    empty store path."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module } from "@gripsack/core";

export default module("built", {
  steps: [{
    id: "build",
    action: { kind: "custom_shell", script: "echo hello-artifact > artifact.txt" },
  }],
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    home = sandbox / ".local/share/gripsack"
    artifacts = list((home / "store").glob("*-built/artifact.txt"))
    assert artifacts, "build step output missing from the store path"
    assert artifacts[0].read_text().strip() == "hello-artifact"


def test_tree_entries_and_prune_on_undeclare(sandbox):
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text('{"theme": "mocha"}\n')
    (confdir / "keymap.json").write_text("[]\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("zed", {
  config: { ...tree("configs/zed", "~/.config/zed") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "zed"
    assert (deployed / "settings.json").exists()
    assert (deployed / "keymap.json").exists()

    # drop a file from the tree -> pruned on next apply
    (confdir / "keymap.json").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "settings.json").exists()
    assert not (deployed / "keymap.json").exists()


def test_owned_prune_on_undeclare(sandbox):
    """Regression: prune-on-undeclare must work for owned symlinks too —
    the recorded hash is the source content, so the tracked_copy hash
    check can never match a symlink (gripsack-exec apply.rs)."""
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    (confdir / "b.txt").write_text("b\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("zed", {
  config: { ...tree("configs/zed", "~/.config/zed", "owned") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "zed"
    assert (deployed / "a.txt").is_symlink()
    assert (deployed / "b.txt").is_symlink()

    (confdir / "b.txt").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "a.txt").is_symlink()
    assert not (deployed / "b.txt").exists()

    # a user file replacing our symlink is never pruned
    (deployed / "a.txt").unlink()
    (deployed / "a.txt").write_text("user edit\n")
    (confdir / "a.txt").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "a.txt").read_text() == "user edit\n"


def test_apply_repo_from_elsewhere(sandbox):
    """The bootstrap story: apply a repo that isn't the cwd."""
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
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
    elsewhere = sandbox / "elsewhere"
    elsewhere.mkdir()
    out = grip("apply", "--host", "testhost", "--repo", str(repo), cwd=elsewhere)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello").is_symlink()

    # git URL form (a local path is a valid clone source)
    git_env = {
        "GIT_AUTHOR_NAME": "t",
        "GIT_AUTHOR_EMAIL": "t@t",
        "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@t",
        "PATH": os.environ["PATH"],
    }
    subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True, env=git_env)
    subprocess.run(["git", "add", "-A"], cwd=repo, check=True, env=git_env)
    subprocess.run(["git", "commit", "--quiet", "-m", "init"], cwd=repo, check=True, env=git_env)
    bare = sandbox / "myenv-remote"
    subprocess.run(["git", "clone", "--quiet", str(repo), str(bare)], check=True, env=git_env)
    out = grip("apply", "--host", "testhost", "--repo", str(bare), cwd=elsewhere)
    assert out.returncode == 0, out.stderr


def test_update_rewrites_lockfile_then_apply_deploys(sandbox):
    """The flake cycle: update moves the lockfile, apply executes it."""
    payload = sandbox / "hello.tar.gz"
    make_tarball(payload, {"bin/hello": b"#!/bin/sh\necho v1\n"})
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
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    lock = repo / "locks" / "testhost.lock"
    assert lock.exists()
    first_pin = lock.read_text()

    # no movement -> unchanged
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "unchanged" in out.stdout
    assert lock.read_text() == first_pin

    # payload changes -> update bumps the pin, apply deploys it
    make_tarball(payload, {"bin/hello": b"#!/bin/sh\necho v2\n"})
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bumped" in out.stdout
    assert lock.read_text() != first_pin

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "applied" in out.stdout


def test_config_tree_gain_repins_and_deploys_from_store(sandbox):
    """A config tree that gains a file under an unmoved transport pin:
    `grip update` moves the pin, a warm-store apply deploys the new
    file FROM THE STORE (never a link into the repo checkout), and a
    cold store re-pins instead of dying on the stale tree hash."""
    payload = make_tarball(sandbox / "tool.tar.gz", {"bin/tool": b"#!/bin/sh\n"})
    confdir = sandbox / "myenv" / "configs" / "tool"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink, tree }} from "@gripsack/core";

export default module("tool", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/tool": symlink("~/.local/bin/tool") }},
  config: {{ ...tree("configs/tool", "~/.config/tool", "owned") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    store = sandbox / ".local/share/gripsack/store"
    deployed_a = sandbox / ".config" / "tool" / "a.conf"
    assert deployed_a.is_symlink()
    assert str(deployed_a.readlink()).startswith(str(store))

    # the tree gains a file: update reports the move, apply deploys it
    (confdir / "b.conf").write_text("b\n")
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bumped" in out.stdout, out.stdout
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed_b = sandbox / ".config" / "tool" / "b.conf"
    assert deployed_b.is_symlink()
    target = str(deployed_b.readlink())
    assert target.startswith(str(store)), f"new file deployed from the repo checkout: {target}"

    # cold store: the pin moved, so this re-pins instead of failing
    shutil.rmtree(store)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config" / "tool" / "b.conf").exists()
