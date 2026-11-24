"""#409: local repos are trusted, remotes are trusted on first use, and the plan
is the guardrail for new hooks, fetch domains and destinations outside ~."""
import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest
from conftest import GRIP, grip, make_workspace_repo, refresh_workspace

HOOK_OUTPUT = '''import { hook, exec, lit } from "@gripsack/core";
export default hook("setup", {trigger: "post_link",
  run: exec({argv: [lit("/bin/sh"), lit("-c"), lit("echo hello %(url)s")]})});
'''
PROFILE = '''import { profile, file, literalText, trackedCopyTo } from "@gripsack/core";
export default profile("main", {hooks: ["setup"], files: [file({
  content: literalText("%(content)s\\n"), destination: trackedCopyTo("%(destination)s")})]});
'''
PLAIN = '''import { profile, file, literalText, trackedCopyTo } from "@gripsack/core";
export default profile("main", {files: [file({
  content: literalText("x\\n"), destination: trackedCopyTo("~/.trust-flow-owned")})]});
'''


URL = "https://downloads.example.test/install.sh"


def workspace(root, source):
    return make_workspace_repo(root, {"main": source})


def declare(repo, *, url=URL, content="x", destination="~/.trust-flow-owned"):
    """Rewrite the hook and profile outputs of a hooked workspace."""
    (repo / "outputs/setup.ts").write_text(HOOK_OUTPUT % {"url": url})
    (repo / "outputs/main.ts").write_text(
        PROFILE % {"content": content, "destination": destination})


def hooked(root, **values):
    repo = make_workspace_repo(root, {"setup": HOOK_OUTPUT % {"url": URL}, "main": PROFILE % {
        "content": "x", "destination": "~/.trust-flow-owned"}})
    declare(repo, **values)
    return repo


def current(sandbox):
    return (sandbox / ".local/share/gripsack/current")


def tty(command):
    probe = subprocess.run(["script", "-qec", "true", "/dev/null"], capture_output=True)
    assert probe.returncode == 0
    return ["script", "-qec", command, "/dev/null"]


def test_local_repo_applies_without_any_approval(sandbox):
    repo = workspace(sandbox / "repo", PLAIN)
    result = grip("apply", cwd=repo, approve=False)
    assert result.returncode == 0, result.stdout + result.stderr
    assert (sandbox / ".trust-flow-owned").read_text() == "x\n"
    home = sandbox / ".local/share/gripsack"
    assert not (home / "trust.toml").exists() and not (home / "known-remotes").exists()
    # Editing the repo needs nothing either: unchanged plain files never prompt.
    (repo / "outputs/main.ts").write_text(PLAIN.replace('"x\\n"', '"y\\n"'))
    again = grip("apply", cwd=repo, approve=False)
    assert again.returncode == 0, again.stdout + again.stderr
    assert (sandbox / ".trust-flow-owned").read_text() == "y\n"


def test_plan_highlights_new_hook_bodies_domains_and_outside_destinations(sandbox):
    repo = hooked(sandbox / "repo", destination=f"{sandbox}/outside-home")
    (repo / "outputs/tool.ts").write_text('''import { pkg, provider, tarball } from "@gripsack/core";
export default pkg("tool", {producer: provider(tarball("https://releases.example.test/tool.tgz")),
  commands: {}, target: {os: "linux", arch: "x86_64"}, layout: {kind: "relocatable"}});
''')
    refresh_workspace(repo)
    plan = grip("plan", cwd=repo, approve=False)
    assert plan.returncode == 0, plan.stdout + plan.stderr
    assert "needs review" in plan.stdout
    assert "hook setup: /bin/sh -c echo hello https://downloads.example.test/install.sh" in plan.stdout
    assert "fetches from releases.example.test" in plan.stdout
    assert f"writes outside ~: {sandbox}/outside-home" in plan.stdout
    structured = grip("plan", "--json", cwd=repo, approve=False)
    assert structured.returncode == 0, structured.stdout + structured.stderr
    review = json.loads(structured.stdout)["needs_review"]
    assert review["domains"] == ["releases.example.test"]
    assert review["outside_home"] == [f"{sandbox}/outside-home"]
    assert len(review["hooks"]) == 1
    # Planning never applies anything.
    assert not current(sandbox).exists()


def test_apply_confirms_only_new_items_and_scripts_need_yes(sandbox):
    repo = hooked(sandbox / "repo")
    refused = grip("apply", cwd=repo, approve=False)
    assert refused.returncode != 0
    assert "re-run with --yes" in refused.stderr, refused.stderr
    assert "hook setup" in refused.stdout
    assert not current(sandbox).exists()
    assert not (sandbox / ".trust-flow-owned").exists()

    applied = grip("apply", "--yes", cwd=repo, approve=False)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    assert "hook setup" in applied.stdout
    assert current(sandbox).exists()

    # The same declarations are no longer new: no prompt, no --yes needed.
    declare(repo, content="z")
    quiet = grip("apply", cwd=repo, approve=False)
    assert quiet.returncode == 0, quiet.stdout + quiet.stderr
    assert "needs review" not in quiet.stdout and "new since" not in quiet.stdout

    # A changed hook body or a new domain is new again.
    declare(repo, url="https://other.example.test/install.sh")
    again = grip("apply", cwd=repo, approve=False)
    assert again.returncode != 0
    assert "other.example.test" in again.stdout


@pytest.mark.skipif(not shutil.which("script"), reason="script(1) not available")
@pytest.mark.parametrize(("answer", "applied"), [(b"y\n", True), (b"n\n", False), (b"\n", False)])
def test_apply_on_a_tty_asks_once_when_something_is_new(sandbox, answer, applied):
    repo = hooked(sandbox / "repo")
    env = dict(os.environ, HOME=str(sandbox), GRIPSACK_HOME=str(sandbox / ".local/share/gripsack"))
    out = subprocess.run(
        tty(f"{GRIP.resolve()} apply"), input=answer, capture_output=True, env=env, cwd=repo, timeout=120,
    )
    transcript = out.stdout.decode(errors="replace")
    assert "apply? [y/N]" in transcript, transcript
    assert (out.returncode == 0) is applied, transcript
    assert current(sandbox).exists() is applied


@pytest.mark.skipif(not shutil.which("script"), reason="script(1) not available")
def test_apply_on_a_tty_never_asks_for_plain_files(sandbox):
    repo = workspace(sandbox / "repo", PLAIN)
    env = dict(os.environ, HOME=str(sandbox), GRIPSACK_HOME=str(sandbox / ".local/share/gripsack"))
    out = subprocess.run(
        tty(f"{GRIP.resolve()} apply"), input=b"", capture_output=True, env=env, cwd=repo, timeout=120,
    )
    transcript = out.stdout.decode(errors="replace")
    assert out.returncode == 0, transcript
    assert "[y/N]" not in transcript


def git(*arguments, cwd):
    result = subprocess.run(
        ["git", "-c", "protocol.file.allow=always", "-c", "user.name=Fixture",
         "-c", "user.email=fixture@example.invalid", *arguments],
        cwd=cwd, env=dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null"),
        capture_output=True, text=True, timeout=30,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def remote_repo(sandbox):
    origin = hooked(sandbox / "origin")
    git("init", "-q", cwd=origin)
    git("add", ".", cwd=origin)
    git("commit", "-qm", "first", cwd=origin)
    return origin, origin.as_uri()


def known(sandbox):
    path = sandbox / ".local/share/gripsack/known-remotes"
    return path.read_text().split() if path.exists() else []


def test_remote_repo_is_trusted_on_first_use_by_origin_url(sandbox):
    origin, url = remote_repo(sandbox)
    # Non-interactive and without --yes: nothing is trusted, nothing is applied.
    refused = grip("check", "--repo", url, cwd=sandbox, approve=False)
    assert refused.returncode != 0
    assert "first use of remote repo" in refused.stdout
    assert "hook setup" in refused.stdout and "downloads.example.test" in refused.stdout
    assert "re-run with --yes" in refused.stderr
    assert known(sandbox) == []

    trusted = grip("check", "--repo", url, "--yes", cwd=sandbox, approve=False)
    assert trusted.returncode == 0, trusted.stdout + trusted.stderr
    assert known(sandbox) == [url]
    listing = grip("trust", "list", approve=False)
    assert listing.stdout.split() == [url]

    # New commits from the same origin never prompt again.
    declare(origin, url="https://new.example.test/x.sh")
    git("commit", "-qam", "second", cwd=origin)
    later = grip("check", "--repo", url, cwd=sandbox, approve=False)
    assert later.returncode == 0, later.stdout + later.stderr
    assert "first use" not in later.stdout

    # Revoking re-arms the prompt.
    removed = grip("trust", "remove", url, approve=False)
    assert removed.returncode == 0, removed.stderr
    assert known(sandbox) == []
    again = grip("check", "--repo", url, cwd=sandbox, approve=False)
    assert again.returncode != 0


def test_a_different_origin_is_a_different_trust_decision(sandbox):
    _, url = remote_repo(sandbox)
    other = sandbox / "other-origin"
    shutil.copytree(sandbox / "origin", other)
    assert grip("check", "--repo", url, "--yes", cwd=sandbox, approve=False).returncode == 0
    refused = grip("check", "--repo", other.as_uri(), cwd=sandbox, approve=False)
    assert refused.returncode != 0
    assert known(sandbox) == [url]


@pytest.mark.skipif(not shutil.which("script"), reason="script(1) not available")
@pytest.mark.parametrize(("answer", "trusted"), [(b"y\n", True), (b"n\n", False)])
def test_remote_first_use_prompts_on_a_tty(sandbox, answer, trusted):
    _, url = remote_repo(sandbox)
    env = dict(os.environ, HOME=str(sandbox), GRIPSACK_HOME=str(sandbox / ".local/share/gripsack"))
    out = subprocess.run(
        tty(f"{GRIP.resolve()} check --repo {url}"), input=answer, capture_output=True,
        env=env, cwd=sandbox, timeout=120,
    )
    transcript = out.stdout.decode(errors="replace")
    assert "trust this remote? [y/N]" in transcript, transcript
    assert (out.returncode == 0) is trusted, transcript
    assert known(sandbox) == ([url] if trusted else [])


def test_stale_trust_state_is_ignored(sandbox):
    home = sandbox / ".local/share/gripsack"
    home.mkdir(parents=True)
    (home / "trust.toml").write_text("version = 2\napproved = []\nlegacy = []\n")
    repo = workspace(sandbox / "repo", PLAIN)
    assert grip("apply", cwd=repo, approve=False).returncode == 0
