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
    approve_fixture,
    grip,
    make_env_repo,
)


def approved_adoption(*args, cwd):
    """Approve both disposable snapshots; --yes never grants source approval."""
    prepared = grip(*args, cwd=cwd)
    assert prepared.returncode == 1, prepared.stdout + prepared.stderr
    resumed = list(args)
    if "--mode" in resumed:
        index = resumed.index("--mode")
        del resumed[index:index + 2]
    result = grip(*resumed, "--resume", cwd=cwd)
    result.stdout = prepared.stdout + result.stdout
    return result

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

    out = approved_adoption(
        "adopt", "~/.config/helix", "--mode", "owned",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    assert (repo / "configs/helix/config.toml").read_text() == 'theme = "gruvbox"\n'
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
    out = approved_adoption("adopt", "~/.config/zed", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (confdir / "settings.json").read_text() == "{}\n"
    assert not (confdir / "settings.json").is_symlink()


def script_tty(command: str) -> list[str]:
    """script(1) argv for a pty, portable across util-linux and BSD
    (macOS): GNU needs -c, BSD takes the command as trailing args."""
    probe = subprocess.run(["script", "-qec", "true", "/dev/null"], capture_output=True)
    if probe.returncode == 0:
        return ["script", "-qec", command, "/dev/null"]
    return ["script", "-q", "/dev/null", "sh", "-c", command]


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
        "PATH": f"{grip_bin.parent}:{os.environ['PATH']}",
    })
    approve_fixture([str(grip_bin), "adopt", "~/.config/helix"], cwd=repo, env=env)
    # Safe menu default, generated-source approval, then apply confirmation.
    out = subprocess.run(
        script_tty("grip adopt ~/.config/helix --host testhost"),
        input=b"\ny\ny\n", capture_output=True, env=env, cwd=repo, timeout=90,
    )
    transcript = out.stdout.decode(errors="replace") + out.stderr.decode(errors="replace")
    assert out.returncode == 0, transcript
    assert (confdir / "config.toml").read_text() == 'theme = "x"\n'
    assert not (confdir / "config.toml").is_symlink()


def test_adopt_refuses_path_outside_home(sandbox):
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "/etc/hosts", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode != 0
    assert "outside your home" in out.stderr


def test_adopt_rejects_invalid_host_before_generating_repo_files(sandbox):
    target = sandbox / ".config" / "demo"
    target.mkdir(parents=True)
    (target / "settings.conf").write_text("untouched\n")
    repo = make_env_repo(sandbox / "myenv", {})
    host_file = repo / "hosts" / "testhost.ts"
    before_host = host_file.read_bytes()
    out = grip(
        "adopt", "~/.config/demo", "--mode", "tracked_copy",
        "--host", "../modules/evil", "--yes", cwd=repo,
    )
    assert out.returncode != 0
    assert "E132" in out.stderr, out.stderr
    assert host_file.read_bytes() == before_host
    assert not (repo / "modules" / "demo.ts").exists()
    assert not (repo / "configs" / "demo").exists()
    assert (target / "settings.conf").read_text() == "untouched\n"


def test_untrusted_adopt_never_inspects_or_generates_repo_files(sandbox):
    """Missing captured-source approval precedes target reads and repo writes."""
    target = sandbox / ".config" / "demo"
    target.mkdir(parents=True)
    (target / "settings.conf").write_text("untouched\n")
    repo = make_env_repo(sandbox / "myenv", {})
    host_file = repo / "hosts" / "testhost.ts"
    original_host = host_file.read_bytes()
    out = grip(
        "adopt", "~/.config/demo", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo, approve=False,
    )
    assert out.returncode != 0
    assert host_file.read_bytes() == original_host
    assert not (repo / "modules" / "demo.ts").exists()
    assert not (repo / "configs" / "demo").exists()
    assert (target / "settings.conf").read_text() == "untouched\n"


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
    out = approved_adoption("adopt", "~/.config/demo", "--mode", "owned",
               "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not (repo / "configs/demo/cache/big.txt").exists()
    assert (repo / "configs/demo/a.txt").read_text() == "a\n"


def test_adopt_merge_mode_manages_one_block(sandbox):
    """merge mode: adopt takes one managed block, and rollback strips
    exactly that block, leaving the original bytes."""
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("export EDITOR=hx\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = approved_adoption(
        "adopt", "~/.bashrc", "--mode", "merge",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    assert "EDITOR=hx" in bashrc.read_text()  # content preserved
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert bashrc.read_text() == "export EDITOR=hx\n"


def test_adopt_refuses_an_already_managed_path(sandbox):
    confdir = sandbox / ".config" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = approved_adoption("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
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
    out = approved_adoption(
        "adopt", "~/.config/other", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    # drift the managed copy — with a global --take-over this would be
    # clobbered; adopt's scoped set contains only the NEW destinations
    drift_target = sandbox / ".config/other/b.txt"
    drift_target.write_text("user edits\n")
    out = approved_adoption("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert drift_target.read_text() == "user edits\n"  # drift preserved


def test_adopt_rollback_keeps_post_adopt_user_edits(sandbox):
    """0015 §4's drift guard: a destination the user changed after
    adopting is theirs — rollback keeps it, prior or not."""
    confdir = sandbox / ".config" / "helix"
    confdir.mkdir(parents=True)
    (confdir / "config.toml").write_text('theme = "gruvbox"\n')
    repo = make_env_repo(sandbox / "myenv", {})
    out = approved_adoption("adopt", "~/.config/helix", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = confdir / "config.toml"
    dest.unlink()
    dest.write_text('theme = "mine now"\n')
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == 'theme = "mine now"\n'


def test_adopt_sanitizes_digit_leading_names(sandbox):
    """0033 R6: a digit-leading dotfile yields a valid TS module —
    check passes on the generated code, no user repair needed."""
    confdir = sandbox / ".config"
    confdir.mkdir(parents=True)
    (confdir / "9lives.conf").write_text("lives=9\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = approved_adoption(
        "adopt", "~/.config/9lives.conf", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_generated_adoption_requires_new_approval_before_resume(sandbox):
    target = sandbox / ".config/demo"
    target.mkdir(parents=True)
    (target / "config").write_text("original\n")
    repo = make_env_repo(sandbox / "repo", {})
    prepared = grip("adopt", "~/.config/demo", "--mode", "owned",
                    "--host", "testhost", "--yes", cwd=repo)
    assert prepared.returncode == 1
    generated = {path.relative_to(repo): path.read_bytes()
                 for path in repo.rglob("*") if path.is_file()}
    assert (target / "config").read_text() == "original\n"
    assert not (target / "config").is_symlink()
    state = sandbox / ".local/share/gripsack"
    assert not (state / "generations").exists()
    denied = grip("adopt", "~/.config/demo", "--host", "testhost",
                  "--yes", "--resume", cwd=repo, approve=False)
    assert denied.returncode == 1
    assert not (state / "generations").exists()
    approved = grip("adopt", "~/.config/demo", "--host", "testhost",
                    "--yes", "--resume", cwd=repo)
    assert approved.returncode == 0, approved.stdout + approved.stderr
    assert (target / "config").is_symlink()
    assert all((repo / path).read_bytes() == content for path, content in generated.items())
    rolled_back = grip("rollback", cwd=repo)
    assert rolled_back.returncode == 0, rolled_back.stderr
    assert not (target / "config").is_symlink()
    assert (target / "config").read_text() == "original\n"


def test_resume_cannot_expand_the_requested_takeover_scope(sandbox):
    target = sandbox / ".config/demo"
    target.mkdir(parents=True)
    (target / "config").write_text("original\n")
    outside = sandbox / ".outside"
    outside.write_text("not adopted\n")
    repo = make_env_repo(sandbox / "repo", {
        "demo": 'import {module,trackedCopy} from "@gripsack/core";\n'
                'export default module("demo", {config: {payload: trackedCopy("~/.outside")}});\n',
    })
    (repo / "payload").write_text("replacement\n")
    result = grip("adopt", "~/.config/demo", "--host", "testhost",
                  "--yes", "--resume", cwd=repo)
    assert result.returncode == 1
    assert outside.read_text() == "not adopted\n"
    assert (target / "config").read_text() == "original\n"
    assert not (sandbox / ".local/share/gripsack/generations").exists()
