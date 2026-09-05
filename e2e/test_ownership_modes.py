"""Ownership-mode e2e: owned/tracked_copy/template/merge — foreign-path
refusals and take-over, drift policy, priors on rollback — split from
test_flow.py; fixture repos come from conftest."""



from conftest import (
    grip,
    make_env_repo,
    make_tarball,
    remove_module,
)



def test_owned_deploy_refuses_foreign_paths_unless_take_over(sandbox):
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
    foreign = sandbox / ".local" / "bin"
    foreign.mkdir(parents=True)
    (foreign / "hello").write_text("system binary\n")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "not deployed by gripsack" in out.stderr

    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (foreign / "hello").is_symlink()


def test_owned_deploy_refuses_foreign_symlinks(sandbox):
    """Review finding E4: a stow-style foreign symlink is exactly the
    path the guard is for — refuse unless --take-over."""
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
    foreign = sandbox / ".local/bin"
    foreign.mkdir(parents=True)
    stow_target = sandbox / "elsewhere/real-hello"
    stow_target.parent.mkdir(parents=True)
    stow_target.write_text("#!/bin/sh\necho stow\n")
    (foreign / "hello").symlink_to(stow_target)

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "not deployed by gripsack" in out.stderr
    assert (foreign / "hello").readlink() == stow_target  # untouched

    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert str(foreign / "hello").endswith("hello")


def test_merge_mode_owns_one_block_in_a_foreign_file(sandbox):
    """merge: gripsack owns exactly one delimited block; everything
    outside the markers is never touched (0001 §3.7)."""
    confdir = sandbox / "myenv" / "configs" / "shell"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text('export PATH="$HOME/.local/bin:$PATH"\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module } from "@gripsack/core";

export default module("shell", {
  config: { "configs/shell/block.sh": merge("~/.bashrc") },
});
""",
    )
    # foreign file with pre-existing content
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("# user stuff\nexport EDITOR=hx\n")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert content.startswith("# user stuff\nexport EDITOR=hx\n")
    assert ">>> gripsack module=shell sha=" in content
    assert 'export PATH="$HOME/.local/bin:$PATH"' in content
    assert "# <<< gripsack module=shell <<<" in content

    # re-apply is satisfied; user drift INSIDE the block self-heals,
    # user content outside is untouched
    healed = content.replace("export PATH", "# user edited\nexport PATH")
    bashrc.write_text(healed + "\n# more user stuff\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert content.count(">>> gripsack module=shell sha=") == 1
    assert "# user edited" not in content
    assert content.endswith("# more user stuff\n")

    # undeclare prunes only the block; the foreign file stays
    remove_module(repo, "hello")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert "gripsack" not in content
    assert "# user stuff" in content
    assert "# more user stuff" in content


def test_template_mode_renders_vars_at_deploy(sandbox):
    """template: {{ name }} placeholders render from entry vars at
    deploy time; undefined variables fail loudly (0001 §3.7)."""
    confdir = sandbox / "myenv" / "configs" / "git"
    confdir.mkdir(parents=True)
    (confdir / "id.toml").write_text(
        'email = "{{ email }}"\nname = "{{ name }}"\nliteral = "{{{{ keep }}"\n'
    )
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, template } from "@gripsack/core";

export default module("git", {
  config: {
    "configs/git/id.toml": template("~/.config/git/id.toml", {
      email: "a@b.c",
      name: "T",
    }),
  },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "git" / "id.toml"
    assert deployed.read_text() == 'email = "a@b.c"\nname = "T"\nliteral = "{{ keep }}"\n'

    # changing a var updates the rendered dest on the next apply
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, template } from "@gripsack/core";

export default module("git", {
  config: {
    "configs/git/id.toml": template("~/.config/git/id.toml", {
      email: "x@y.z",
      name: "T",
    }),
  },
});
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert 'email = "x@y.z"' in deployed.read_text()

    # an undefined variable fails at apply, never silently empty
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, template } from "@gripsack/core";

export default module("git", {
  config: {
    "configs/git/id.toml": template("~/.config/git/id.toml", {
      email: "x@y.z",
    }),
  },
});
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "undefined variable" in out.stderr


def test_tracked_copy_drift_is_kept_never_clobbered(sandbox):
    """The killer drift policy (0001 §3.7): a user edit inside a
    tracked_copy destination is detected and KEPT — gripsack never
    silently overwrites it (review finding G: this path had zero
    coverage)."""
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text('{"theme": "mocha"}\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("zed", {
  config: { "configs/zed/settings.json": trackedCopy("~/.config/zed/settings.json") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".config" / "zed" / "settings.json"

    # user edits the deployed file (zed rewrites its own config) — the
    # next apply detects drift and KEEPS it
    dest.write_text('{"theme": "nord", "user": true}\n')
    (confdir / "settings.json").write_text('{"theme": "latte"}\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == '{"theme": "nord", "user": true}\n'

    # a second apply without hand-resolution must NOT promote the
    # drift to authority (0029 §2 — the 0.24 promotion bug: the
    # recorded observed hash made the next apply overwrite it)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == '{"theme": "nord", "user": true}\n', (
        "preserved drift must survive repeated applies"
    )

    # convergence is explicit: hand-write the DESIRED content and the
    # next apply reads managed+satisfied (observation is never consent)
    dest.write_text('{"theme": "latte"}\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == '{"theme": "latte"}\n'
    # and now the repo drives again
    (confdir / "settings.json").write_text('{"theme": "frappe"}\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == '{"theme": "frappe"}\n'


def test_rollback_restores_template_rendered_and_merge_block(sandbox):
    """rollback through the ONE engine (0001 §3.5, review verification):
    template destinations get the previous generation's RENDERED bytes
    (re-rendered with recorded vars), and merge entries re-upsert only
    the block — the foreign file's other content survives."""
    confdir = sandbox / "myenv" / "configs" / "app"
    confdir.mkdir(parents=True)
    (confdir / "id.toml").write_text('email = "{{ email }}"\n')
    (confdir / "block.sh").write_text("export A=1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module, template } from "@gripsack/core";

export default module("app", {
  config: {
    "configs/app/id.toml": template("~/.config/app/id.toml", { email: "a@b.c" }),
    "configs/app/block.sh": merge("~/.bashrc"),
  },
});
""",
    )
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("# user stuff\nexport EDITOR=hx\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    # generation 2: new template content and new block content
    (confdir / "id.toml").write_text('email = "rendered-v2"\nname = "{{ email }}"\n')
    (confdir / "block.sh").write_text("export A=2\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "rendered-v2" in (sandbox / ".config/app/id.toml").read_text()
    assert "export A=2" in bashrc.read_text()

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    # template: back to the previous generation's rendered bytes
    assert (sandbox / ".config/app/id.toml").read_text() == 'email = "a@b.c"\n'
    # merge: the block reverted, the foreign content untouched
    content = bashrc.read_text()
    assert content.startswith("# user stuff\nexport EDITOR=hx\n")
    assert "export A=1" in content
    assert "export A=2" not in content


def test_deploy_refuses_destination_resolving_into_repo(sandbox):
    """A destination whose ancestor is a symlink into the env repo
    turns a deploy into a delete of the module's own source — refuse."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("scripts", {
  config: { ".claude/scripts/deploy.sh": trackedCopy("~/.claude-config/scripts/deploy.sh") },
});
""",
    )
    source = repo / ".claude" / "scripts"
    source.mkdir(parents=True)
    (source / "deploy.sh").write_text("#!/bin/sh\necho real\n")
    (sandbox / ".claude-config").symlink_to(repo / ".claude")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, out.stdout
    assert "resolves inside the env repo" in out.stderr
    # the module's source survived untouched
    assert (source / "deploy.sh").read_text() == "#!/bin/sh\necho real\n"


def test_owned_replaces_a_stale_symlink_into_the_repo(sandbox):
    """F2 regression: an `owned` destination that is itself a symlink
    into the repo — an artifact of an older gripsack that deployed
    config straight from the checkout — is prior state gripsack may
    replace (nothing is written THROUGH it). The containment guard
    used to refuse forever with an error that pointed at the module
    instead of the stale link: a hard stop on the first apply after
    upgrading, with no in-product way out."""
    payload = make_tarball(
        sandbox / "owned.tar.gz", {"bin/tool": b"#!/bin/sh\necho tool\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("m", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/tool": symlink("~/.local/bin/tool") }},
}});
""",
    )
    # the stale artifact: an owned destination pointing INTO the repo,
    # as an old gripsack would have written it
    dest = sandbox / ".local/bin/tool"
    dest.parent.mkdir(parents=True)
    (repo / "stale-target").write_text("old checkout artifact\n")
    dest.symlink_to(repo / "stale-target")

    # without --take-over the normal owned drift guard answers — with
    # the mechanism and the way out named, not the containment error
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "not deployed by gripsack" in out.stderr
    assert "take-over" in out.stderr
    assert dest.is_symlink() and dest.resolve() == (repo / "stale-target")

    # with --take-over the stale link is prior state: replaced by the
    # store link, original target untouched
    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.is_symlink(), "owned semantics replace the link, not follow it"
    assert "store" in str(dest.resolve()), out.stdout
    assert (repo / "stale-target").read_text() == "old checkout artifact\n"


def test_write_through_mode_refuses_repo_symlink_with_a_hint(sandbox):
    """The same stale link under a write-THROUGH mode (tracked_copy)
    still refuses — writing would land in the checkout — but the error
    names the mechanism and the way out."""
    confdir = sandbox / "myenv" / "configs" / "app"
    confdir.mkdir(parents=True)
    (confdir / "app.conf").write_text("key = 1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("m", {
  config: { "configs/app/app.conf": trackedCopy("~/.config/app/app.conf") },
});
""",
    )
    dest = sandbox / ".config/app/app.conf"
    dest.parent.mkdir(parents=True)
    (repo / "stale-conf").write_text("stale\n")
    dest.symlink_to(repo / "stale-conf")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "resolves inside the env repo" in out.stderr
    assert "symlink into the repo" in out.stderr, out.stderr
    assert (repo / "stale-conf").read_text() == "stale\n"


def test_adoption_origin_survives_generations_and_gc(sandbox):
    """0029 §1: the pre-adoption origin rides the whole ownership
    epoch — carried through satisfied applies and content updates,
    pinned against gc, restored on undeclare. The 0.24 lineage bug
    dropped `prior` on the next ordinary apply."""
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
    # adopt snapshots the live file INTO the repo — the destination
    # keeps its bytes through the adoption apply
    assert original.read_text() == "ORIGINAL\n"

    # ordinary satisfied apply + a content update: the origin must
    # ride along (0.24 dropped it here)
    grip("apply", "--host", "testhost", cwd=repo)
    # adopt snapshots into configs/<module>/ — find where it landed
    adopted = next((repo / "configs").rglob("a.toml"))
    adopted.write_text("v2\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert original.read_text() == "v2\n", f"{out.stdout}\n{out.stderr}"

    # gc the old history — the origin's blob must stay pinned
    user_conf = sandbox / ".config/gripsack"
    user_conf.mkdir(parents=True)
    (user_conf / "config.toml").write_text("[settings]\nkeep_generations = 0\n")
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr

    # undeclare the ADOPTED module (not the scaffold) — the ORIGINAL
    # comes back
    remove_module(repo, "a-toml")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert original.read_text() == "ORIGINAL\n", original.read_text()


def test_foreign_file_is_never_promoted_by_observation(sandbox):
    """0029 §2: a never-managed foreign file stays foreign across
    repeated applies — observing it twice is not consent."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("repo\n")
    foreign = sandbox / ".config/demo/a.toml"
    foreign.parent.mkdir(parents=True)
    foreign.write_text("FOREIGN\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
""",
    )
    for _ in range(3):
        out = grip("apply", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert foreign.read_text() == "FOREIGN\n"
        assert "kept" in out.stdout


def test_undeclaring_a_drift_kept_module_keeps_the_file(sandbox):
    """0029 (unlisted consequence): the drift-keep path recorded the
    observed hash, so undeclare's intact check matched — and prune
    DELETED the user's drifted file. Preserved-drift entries are now
    never pruned."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("repo\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".config/demo/a.toml"
    dest.write_text("user drift\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "kept" in out.stdout

    remove_module(repo, "hello")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == "user drift\n", "prune must not delete preserved drift"


def test_foreign_symlink_at_a_copy_destination_refuses(sandbox):
    """0029 §5: a symlink at a tracked-copy destination is a foreign
    OBJECT — never silently absent, never followed. Take-over captures
    the link as the prior."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("repo\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
""",
    )
    dest = sandbox / ".config/demo/a.toml"
    dest.parent.mkdir(parents=True)
    dest.symlink_to("/elsewhere/foreign.toml")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.is_symlink(), "the foreign link must survive"
    assert "kept" in out.stdout

    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == "repo\n"

    # and the link is the recorded prior — undeclare restores it
    remove_module(repo, "hello")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.is_symlink() and dest.readlink() == __import__("pathlib").Path("/elsewhere/foreign.toml")


def test_merge_block_carries_its_content_hash(sandbox):
    """The open marker embeds the block's content sha: hand-edits
    inside the markers are detectable from the file alone, and the
    re-apply says so while self-healing the block."""
    confdir = sandbox / "myenv" / "configs" / "shell"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text('export SINK=1\n')
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
    content = bashrc.read_text()
    assert "sha=" in content and ">>> gripsack module=shell sha=" in content

    # a hand edit inside the markers — the sha no longer describes
    # the content; the next apply regenerates and says so
    bashrc.write_text(content.replace("SINK=1", "SINK=2"))
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "hand-edited block regenerated" in out.stdout, out.stdout
    assert "SINK=1" in bashrc.read_text(), "the block self-heals"

def test_duplicate_merge_blocks_are_reconciled_and_reported(sandbox):
    """A module owns EVERY block carrying its name: a duplicate is
    reconciled down to one and the removal is NAMED in the report —
    never invisible in steady state, never silently deleted as a side
    effect of an unrelated drift repair (0.21.1 review round)."""
    confdir = sandbox / "myenv" / "configs" / "shell"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text("export SINK=1\n")
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
    content = bashrc.read_text()

    # a byte-identical duplicate block, user content interleaved
    block_start = content.index("# >>> gripsack module=shell")
    block_end = content.index("# <<< gripsack module=shell <<<")
    block_end = content.index("\n", block_end) + 1
    block = content[block_start:block_end]
    bashrc.write_text(
        content + "\n# a user's own note below\n\n" + block + "\nexport USER_OWN_TAIL=keepme\n"
    )

    # steady state with an intact first block: the duplicate must
    # still be seen, reconciled, and NAMED — before the fix this
    # reported "block unchanged" and left both blocks
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "removed 1 duplicate block" in out.stdout, out.stdout
    healed = bashrc.read_text()
    assert healed.count("# >>> gripsack module=shell") == 1
    assert "# a user's own note below" in healed
    assert "export USER_OWN_TAIL=keepme" in healed

    # the next apply is satisfied — reconciliation converges
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "block unchanged" in out.stdout, out.stdout

    # a tampered SECOND block is just as visible as a tampered first:
    # duplicate again, edit inside the copy only — before the fix the
    # content-hash guarantee held only for whichever block came first
    content = bashrc.read_text()
    block_start = content.index("# >>> gripsack module=shell")
    block_end = content.index("# <<< gripsack module=shell <<<")
    block_end = content.index("\n", block_end) + 1
    block = content[block_start:block_end]
    tampered = block.replace("SINK=1", "SINK=2")
    bashrc.write_text(content + "\n" + tampered)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "removed 1 duplicate block" in out.stdout, out.stdout
    final = bashrc.read_text()
    assert final.count("# >>> gripsack module=shell") == 1
    assert "SINK=2" not in final
