"""Destination admission, permission identity and ownership lineage."""

from conftest import grip, make_env_repo, refresh_host, remove_module


def test_duplicate_destination_is_a_check_time_error(sandbox):
    """E111 (N2): two modules may not declare the same destination —
    a deploy race in parallel and a lie for why-owns."""
    confdir = sandbox / "myenv" / "configs" / "x"
    confdir.mkdir(parents=True)
    (confdir / "same.conf").write_text("x\n")
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "one": """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: { "configs/x/same.conf": trackedCopy("~/.out/same.conf") },
});
""",
            "two": """
import { module, trackedCopy } from "@gripsack/core";

export default module("two", {
  config: { "configs/x/same.conf": trackedCopy("~/.out/same.conf") },
});
""",
        },
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E111" in out.stderr
    assert "same.conf" in out.stderr


def test_destination_aliases_are_rejected_before_mutation(sandbox):
    """0030 §P0-1: `~/x` and `$HOME/x` are one physical destination —
    and a symlinked ancestor collapses to one too. Both must fail at
    check/apply time, never double-transition one object."""
    confdir = sandbox / "myenv" / "configs" / "x"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    home = sandbox
    alias = home / ".config/aliased/x.conf"
    alias.parent.mkdir(parents=True)

    repo = make_env_repo(
        sandbox / "myenv",
        {
            "one": """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: { "configs/x/a.conf": trackedCopy("~/.config/aliased/x.conf") },
});
""",
            "two": f"""
import {{ module, trackedCopy }} from "@gripsack/core";

export default module("two", {{
  config: {{ "configs/x/a.conf": trackedCopy("{alias}") }},
}});
""",
        },
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E119" in out.stderr, out.stderr
    assert "resolve to the same path" in out.stderr, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E119" in out.stderr
    assert "resolve to the same path" in out.stderr

    # and a symlinked ancestor: ~/config-link -> ~/.config
    (home / "config-link").symlink_to(home / ".config")
    (sandbox / "myenv2").mkdir(exist_ok=True)
    (sandbox / "myenv2" / "configs" / "x").mkdir(parents=True, exist_ok=True)
    (sandbox / "myenv2" / "configs" / "x" / "a.conf").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv2",
        {
            "one": """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: { "configs/x/a.conf": trackedCopy("~/.config/aliased/x.conf") },
});
""",
            "two": """
import { module, trackedCopy } from "@gripsack/core";

export default module("two", {
  config: { "configs/x/a.conf": trackedCopy("~/config-link/aliased/x.conf") },
});
""",
        },
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E119" in out.stderr
    assert "resolve to the same path" in out.stderr, out.stderr


def test_same_module_duplicate_destination_is_e111(sandbox):
    """0030 §P0-1: E111's same-module suppression is gone — two
    declarations of one destination in ONE module would double-journal
    it, the second entry overwriting the first's true prior."""
    confdir = sandbox / "myenv" / "configs" / "x"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    (confdir / "b.conf").write_text("b\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: {
    "configs/x/a.conf": trackedCopy("~/.out/same.conf"),
    "configs/x/b.conf": trackedCopy("~/.out/same.conf"),
  },
});
""",
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E111" in out.stderr


def test_executable_tracked_copy_stays_coherent(sandbox):
    """0030 §H3 (Option A): a tracked copy manages executability — a
    fresh 0755 deploy lands executable, the next apply is satisfied,
    and an exec-bit change from the repo applies."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    script = confdir / "tool.sh"
    script.write_text("#!/bin/sh\necho v1\n")
    script.chmod(0o755)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/tool.sh": trackedCopy("~/.config/demo/tool.sh") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    import os

    dest = sandbox / ".config/demo/tool.sh"
    assert os.stat(dest).st_mode & 0o111, "the exec bit must land"

    # the next apply is satisfied — the exec-aware identity agrees
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert "unchanged" in out.stdout, out.stdout

    # the repo drops the exec bit — the update applies it
    script.chmod(0o644)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not os.stat(dest).st_mode & 0o111, "the exec update applies"


def test_chmod_only_drift_is_preserved_and_warned(sandbox):
    """0031: a chmod with no content change is drift — the mode-aware
    identity reads it, the next apply preserves and warns, and a
    repo-side exec change still applies (the update path)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    import os

    dest = sandbox / ".config/demo/a.conf"
    dest.chmod(0o600)

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "drifted" in out.stdout, out.stdout
    assert oct(os.stat(dest).st_mode & 0o777) == "0o600", (
        "chmod-only drift is preserved, never silently reverted"
    )


def test_rollback_restores_the_exact_mode(sandbox):
    """0031: rollback restores the recorded mode, not just bytes.
    Generation 1 is executable, generation 2 is the same bytes with
    the exec bit dropped; rolling back to 1 must flip the live file
    back to 0755 (content is identical — only the mode can differ)."""
    import os

    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    script = confdir / "tool.sh"
    script.write_text("#!/bin/sh\necho v1\n")
    script.chmod(0o755)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/tool.sh": trackedCopy("~/.local/bin/tool.sh") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    # generation 2: same content, exec bit dropped — a mode-only update
    script.chmod(0o644)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".local/bin/tool.sh"
    assert not os.stat(dest).st_mode & 0o111

    out = grip("rollback", "1", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert oct(os.stat(dest).st_mode & 0o777) == "0o755", (
        "the recorded mode is restored exactly"
    )


def test_rename_keeps_full_lineage_authority(sandbox):
    """0030 §H4: renaming a module with a content change is an
    authorized UPDATE (not preserved-as-foreign), and undeclare
    restores the original adoption origin."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("v1\n")
    original = sandbox / ".config/demo/a.toml"
    original.parent.mkdir(parents=True)
    original.write_text("ORIGINAL\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.config/demo/a.toml", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr

    # rename the module AND change the content
    adopted = repo / "configs" / "a-toml" / "a.toml"
    adopted.write_text("v2\n")
    (repo / "modules" / "a-toml.ts").unlink()
    (repo / "modules" / "renamed.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("renamed", {
  config: { "configs/a-toml/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
"""
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "updated" in out.stdout, out.stdout
    assert original.read_text() == "v2\n"

    remove_module(repo, "renamed")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert original.read_text() == "ORIGINAL\n"


def test_double_takeover_keeps_the_first_origin(sandbox):
    """0030 §H5: take-over RETAINS the epoch's origin — a second
    --take-over absorbs content but does not rebase the restore
    point. Undeclare restores the FIRST pre-adoption state."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("A\n")
    original = sandbox / ".config/demo/a.toml"
    original.parent.mkdir(parents=True)
    original.write_text("ORIGINAL\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.config/demo/a.toml", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr

    # the app drifts; a second take-over absorbs B over U
    adopted = repo / "configs" / "a-toml" / "a.toml"
    original.write_text("U\n")
    adopted.write_text("B\n")
    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert original.read_text() == "B\n"

    remove_module(repo, "a-toml")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert original.read_text() == "ORIGINAL\n", (
        "the epoch's FIRST origin must survive a second take-over"
    )


def test_exec_copy_rollback_restores_v1(sandbox):
    """0033 (review): two versions of a 0755 tracked copy; rollback to
    generation 1 restores v1's bytes AND the exec bit. (0.27 compared
    rollback identities across domains and kept gen 2.)"""
    import os

    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    script = confdir / "tool.sh"
    script.write_text("#!/bin/sh\necho v1\n")
    script.chmod(0o755)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/tool.sh": trackedCopy("~/.local/bin/tool.sh") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    script.write_text("#!/bin/sh\necho v2\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".local/bin/tool.sh"
    assert dest.read_text().endswith("v2\n")
    out = grip("rollback", "1", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text().endswith("v1\n"), dest.read_text()
    assert os.stat(dest).st_mode & 0o111, "exec bit restored too"


def test_takeover_preserves_a_private_files_mode(sandbox):
    """0033 R1: adopting a 0600 secret keeps it 0600 — the live file
    AND its prior blob. Adoption is not a fresh deploy."""
    import os

    secret = sandbox / ".config/demo/secret.conf"
    secret.parent.mkdir(parents=True)
    secret.write_text("token=hunter2\n")
    secret.chmod(0o600)
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.config/demo/secret.conf", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    assert oct(os.stat(secret).st_mode & 0o777) == "0o600", (
        "take-over must not widen a private file"
    )
    prior_dir = sandbox / ".local/share/gripsack/prior"
    assert oct(os.stat(prior_dir).st_mode & 0o777) == "0o700", (
        "the prior store is owner-only"
    )
    for blob in prior_dir.iterdir():
        assert oct(os.stat(blob).st_mode & 0o777) == "0o600", (
            "a backed-up secret is 0600"
        )


def test_preserved_copy_blocks_a_mode_switch_to_owned(sandbox):
    """0033 R7: a tracked copy that preserved user drift, redeclared
    as an owned symlink — the apply must REFUSE, not overwrite the
    user's edit. Preserved drift authorizes nothing, including a
    mode change."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    dest = sandbox / ".config/demo/a.conf"
    dest.parent.mkdir(parents=True)
    dest.write_text("original\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.config/demo/a.conf", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr

    # the user edits; the next apply preserves (drift)
    dest.write_text("user edit\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "drifted" in out.stdout, out.stdout

    # redeclare as an owned symlink — must refuse, not clobber
    (repo / "modules" / "a-conf.ts").write_text(
        """
import { module, symlink } from "@gripsack/core";

export default module("a-conf", {
  install: { "configs/demo/a.conf": symlink("~/.config/demo/a.conf") },
});
"""
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "the mode switch must refuse over preserved drift"
    assert "not deployed by gripsack" in out.stderr, out.stderr
    assert dest.read_text() == "user edit\n", "the user's edit stands"


def test_spelling_change_preserves_the_file_and_its_lineage(sandbox):
    """0035 F1: changing only a destination's SPELLING (~/x → $HOME/x)
    must not prune the file — ownership identity is canonical, never
    the spelling."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "value.conf").write_text("v1\n")
    home = sandbox
    absolute = home / ".config/example/value.conf"

    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/value.conf": trackedCopy("~/.config/example/value.conf") },
});
"""
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert absolute.read_text() == "v1\n"

    # the reviewer's repro: same file, absolute spelling
    (repo / "modules" / "demo.ts").write_text(
        f"""
import {{ module, trackedCopy }} from "@gripsack/core";

export default module("demo", {{
  config: {{ "configs/demo/value.conf": trackedCopy("{absolute}") }},
}});
"""
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert absolute.exists(), "a spelling change must never delete the file"
    assert absolute.read_text() == "v1\n"

    # and why-owns resolves the spelling either way
    out = grip("why-owns", "~/.config/example/value.conf", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "demo" in out.stdout


def test_merge_block_owner_rename_leaves_no_ghost(sandbox):
    """0026 §2b: renaming the module that owns a merge block must move
    the block — the old module's block is pruned (block ownership is
    per (module, dest)), not left as an unowned ghost beside the new
    one."""
    confdir = sandbox / "myenv" / "configs" / "shell"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text("export RENAMED=1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module } from "@gripsack/core";

export default module("shell", {
  config: { "configs/shell/block.sh": merge("~/.bashrc") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    bashrc = sandbox / ".bashrc"
    assert "module=shell" in bashrc.read_text()

    (repo / "modules" / "hello.ts").write_text(
        """
import { merge, module } from "@gripsack/core";

export default module("terminal", {
  config: { "configs/shell/block.sh": merge("~/.bashrc") },
});
"""
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert "module=terminal" in content
    assert "module=shell" not in content, f"ghost block left behind: {content}"
    assert content.count("export RENAMED=1") == 1
