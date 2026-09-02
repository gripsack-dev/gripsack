"""Adopt flows e2e (plan/0015): non-interactive defaults, the TTY menu,
never-clobber rules, scoped take-over — split from test_flow.py;
fixture repos come from conftest."""



import os
import shutil
import stat
import subprocess

import pytest
from conftest import (
    GRIP,
    grip,
    make_env_repo,
)



def test_adopt_end_to_end_restores_originals(sandbox):
    """0015 §6: adopt generates the module, manages the destination,
    and rollback to the baseline generation restores the ORIGINAL
    real files — bytes and permission bits."""
    confdir = sandbox / ".config" / "helix"
    confdir.mkdir(parents=True)
    original = confdir / "config.toml"
    original.write_text('theme = "gruvbox"\n')
    original_mode = stat.S_IMODE(original.stat().st_mode)
    (confdir / "languages.toml").write_text("[editor]\n")
    repo = make_env_repo(sandbox / "myenv", {})

    out = grip(
        "adopt", "~/.config/helix", "--mode", "owned",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    assert "owned" in out.stdout
    assert (repo / "configs/helix/config.toml").read_text() == 'theme = "gruvbox"\n'
    assert "tree(" in (repo / "modules/helix.ts").read_text()
    assert "helix" in (repo / "hosts/testhost.ts").read_text()
    assert original.is_symlink()  # managed now

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not original.is_symlink()
    assert original.read_text() == 'theme = "gruvbox"\n'
    assert stat.S_IMODE(original.stat().st_mode) == original_mode


def test_adopt_non_interactive_takes_the_safe_default(sandbox):
    """0015 §7 S1: no tables, no guessing — with no TTY to ask, adopt
    takes tracked_copy and SAYS it chose a default."""
    confdir = sandbox / ".config" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text("{}\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "~/.config/zed", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "tracked_copy" in out.stderr or "tracked_copy" in out.stdout
    assert "safe default" in out.stderr
    assert '"tracked_copy"' in (repo / "modules/zed.ts").read_text()


def test_adopt_menu_selects_on_a_tty(sandbox):
    """The interactive menu (0015 §7 S1): bare enter takes the
    highlighted safe default (tracked_copy)."""
    if not shutil.which("script"):
        pytest.skip("script(1) not available")
    confdir = sandbox / ".config" / "helix"
    confdir.mkdir(parents=True)
    (confdir / "config.toml").write_text("theme = \"x\"\n")
    repo = make_env_repo(sandbox / "myenv", {})
    grip_bin = GRIP.resolve()
    env = dict(os.environ)
    env.update({
        "HOME": str(sandbox),
        "GRIPSACK_HOME": str(sandbox / ".local/share/gripsack"),
        "GRIPSACK_TRUST_ALL": "1",
        "PATH": f"{grip_bin.parent}:{os.environ['PATH']}",
    })
    # bare enter on the menu, 'y' at the apply confirm
    out = subprocess.run(
        ["script", "-qec", "grip adopt ~/.config/helix --host testhost", "/dev/null"],
        input=b"\ny\n", capture_output=True, env=env, cwd=repo, timeout=90,
    )
    transcript = out.stdout.decode(errors="replace") + out.stderr.decode(errors="replace")
    assert "how should gripsack own these files?" in transcript
    assert "tracked_copy" in transcript
    assert '"tracked_copy"' in (repo / "modules/helix.ts").read_text()


def test_adopt_refuses_path_outside_home(sandbox):
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "/etc/hostname", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode != 0
    assert "outside your home" in out.stderr


def test_adopt_refuses_to_clobber_the_repo(sandbox):
    """0015 §7 S4: the never-clobber rule covers the repo too."""
    confdir = sandbox / ".config" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_env_repo(sandbox / "myenv", {})
    (repo / "modules/demo.ts").write_text("// hand-written, do not touch\n")
    out = grip("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode != 0
    assert "refusing to overwrite" in out.stderr
    assert (repo / "modules/demo.ts").read_text() == "// hand-written, do not touch\n"


def test_adopt_does_not_follow_directory_symlinks(sandbox):
    """0015 §7 S2: a dir symlink inside the adopted tree must not pull
    an arbitrary tree into the repo — it's skipped and reported."""
    confdir = sandbox / ".config" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    elsewhere = sandbox / "elsewhere"
    elsewhere.mkdir()
    (elsewhere / "big.txt").write_text("x" * 1000)
    (confdir / "cache").symlink_to(elsewhere, target_is_directory=True)
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "~/.config/demo", "--mode", "owned",
               "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "not followed" in out.stdout
    assert not (repo / "configs/demo/cache/big.txt").exists()
    assert (repo / "configs/demo/a.txt").read_text() == "a\n"


def test_adopt_merge_mode_manages_one_block(sandbox):
    """merge mode: adopt takes one managed block, and rollback strips
    exactly that block, leaving the original bytes."""
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("export EDITOR=hx\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.bashrc", "--mode", "merge",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    assert "merge" in out.stdout
    assert "managed block" in out.stdout
    assert "EDITOR=hx" in bashrc.read_text()  # content preserved
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert bashrc.read_text() == "export EDITOR=hx\n"


def test_adopt_refuses_an_already_managed_path(sandbox):
    confdir = sandbox / ".config" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode != 0
    assert 'already managed by module "demo"' in out.stderr


def test_adopt_take_over_is_scoped(sandbox):
    """0015 §3: the adopt apply may absorb exactly the adopted
    destinations — unrelated drift is never clobbered."""
    drifted = sandbox / ".config" / "demo"
    drifted.mkdir(parents=True)
    (drifted / "a.txt").write_text("a\n")
    other = sandbox / ".config" / "other"
    other.mkdir(parents=True)
    (other / "b.txt").write_text("b\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.config/other", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    # drift the managed copy — with a global --take-over this would be
    # clobbered; adopt's scoped set contains only the NEW destinations
    drift_target = sandbox / ".config/other/b.txt"
    drift_target.write_text("user edits\n")
    out = grip("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert drift_target.read_text() == "user edits\n"  # drift preserved


def test_adopt_rollback_keeps_post_adopt_user_edits(sandbox):
    """0015 §4's drift guard: a destination the user changed after
    adopting is theirs — rollback keeps it, prior or not."""
    confdir = sandbox / ".config" / "helix"
    confdir.mkdir(parents=True)
    (confdir / "config.toml").write_text('theme = "gruvbox"\n')
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "~/.config/helix", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = confdir / "config.toml"
    dest.unlink()
    dest.write_text('theme = "mine now"\n')
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == 'theme = "mine now"\n'
