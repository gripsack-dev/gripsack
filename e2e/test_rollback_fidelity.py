"""Rollback fidelity (plan/0056 K2, F2 step 8; design review finding 2).

Rollback restores, per destination, exactly the identity the target
generation recorded (bytes plus mode, or link target) from retained store
bytes, or refuses before it flips. It never writes into an existing
generation (FND-2). The tests that edit a retained manifest stand in for
older receipts (no recorded mode, `template` mode with recorded variables)
and for a record its retained bytes no longer reproduce."""

import hashlib
import json
import os
from pathlib import Path

from conftest import grip, make_workspace_repo, native_target

HOME_STATE = Path(".local/share/gripsack")


def state(sandbox: Path) -> Path:
    return sandbox / HOME_STATE


def selected(sandbox: Path) -> int:
    return json.loads((state(sandbox) / "current/manifest.json").read_text())["number"]


def manifest_path(sandbox: Path, number: int) -> Path:
    return state(sandbox) / "generations" / str(number) / "manifest.json"


def rewrite_entry(sandbox: Path, number: int, to: str, **fields) -> dict:
    """Rewrite one retained entry in place (a stand-in for an older receipt)."""
    path = manifest_path(sandbox, number)
    manifest = json.loads(path.read_text())
    for module in manifest["modules"].values():
        for entry in module["entries"]:
            if entry["to"] == to:
                for key, value in fields.items():
                    if value is None:
                        entry.pop(key, None)
                    else:
                        entry[key] = value
                path.write_text(json.dumps(manifest))
                return entry
    raise AssertionError(f"no entry for {to}")


def entry(sandbox: Path, number: int, to: str) -> dict:
    manifest = json.loads(manifest_path(sandbox, number).read_text())
    return next(
        e for module in manifest["modules"].values() for e in module["entries"] if e["to"] == to
    )


def file_identity(data: bytes, mode: int) -> str:
    """`canonical_bytes_identity`: the copy/template manifest domain."""
    return hashlib.sha256(b"file\0" + b"\x01" + mode.to_bytes(4, "little") + data).hexdigest()


def bytes_hash(data: bytes) -> str:
    """`canonical_bytes_hash`: a pre-0043 template receipt's bytes-only hash."""
    return hashlib.sha256(b"file\0" + b"\x00" + data).hexdigest()


def tree_digest(directory: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(directory.rglob("*")):
        digest.update(str(path.relative_to(directory)).encode())
        if path.is_file():
            digest.update(path.read_bytes())
    return digest.hexdigest()


def template_repo(sandbox: Path) -> tuple[Path, Path]:
    configs = sandbox / "myenv" / "configs"
    configs.mkdir(parents=True)
    (configs / "id.toml").write_text('email = "{{ email }}"\n')
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, profile, repoFile, templateFile, trackedCopyTo } from "@gripsack/core";

export default profile("app", { files: [
    file({ source: repoFile("configs/id.toml"), content: templateFile({ email: "a@b.c" }), destination: trackedCopyTo("~/.config/app/id.toml") })
  ] });
""",
    )
    return repo, configs


def apply(repo: Path) -> None:
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_rollback_restores_the_rendered_bytes_after_the_template_source_changed(sandbox):
    """R1 (refuted for current generations): a workspace template renders into
    the store at preparation; rollback copies the target's retained rendered
    bytes and lands exactly the recorded identity."""
    repo, configs = template_repo(sandbox)
    apply(repo)
    dest = sandbox / ".config/app/id.toml"
    first = dest.read_bytes()
    (configs / "id.toml").write_text('name = "{{ email }}"\nversion = 2\n')
    apply(repo)
    assert dest.read_bytes() != first

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_bytes() == first == b'email = "a@b.c"\n'
    recorded = entry(sandbox, 1, "~/.config/app/id.toml")
    assert file_identity(dest.read_bytes(), dest.stat().st_mode & 0o7777) == recorded["hash"]


def test_rollback_restores_the_recorded_mode(sandbox):
    """R2: the restored file carries the target's recorded mode exactly."""
    configs = sandbox / "myenv" / "configs"
    configs.mkdir(parents=True)
    script = configs / "tool.sh"
    script.write_text("echo one\n")
    script.chmod(0o644)
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("tool", { files: [
    file({ source: repoFile("configs/tool.sh"), content: identity(), destination: trackedCopyTo("~/.local/bin/tool.sh") })
  ] });
""",
    )
    apply(repo)
    dest = sandbox / ".local/bin/tool.sh"
    assert dest.stat().st_mode & 0o7777 == 0o644
    script.chmod(0o755)
    apply(repo)
    assert dest.stat().st_mode & 0o7777 == 0o755

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.stat().st_mode & 0o7777 == 0o644
    assert dest.read_bytes() == b"echo one\n"


def test_rollback_refuses_a_receipt_its_retained_bytes_do_not_reproduce(sandbox):
    """R2: an older receipt without a recorded mode whose identity says 0600.
    Before: rollback guessed 0644 from the payload, wrote it, flipped, and the
    target generation claimed an identity nothing on disk had. Now: refused
    before any write, the current generation stays selected."""
    repo, configs = template_repo(sandbox)
    apply(repo)
    dest = sandbox / ".config/app/id.toml"
    first = dest.read_bytes()
    (configs / "id.toml").write_text('email = "{{ email }}"\nversion = 2\n')
    apply(repo)
    second = dest.read_bytes()
    # generation 1 as an older receipt: no recorded mode, an identity at 0600
    rewrite_entry(
        sandbox, 1, "~/.config/app/id.toml",
        hash=file_identity(first, 0o600), file_mode=None, source_executable=None,
    )

    out = grip("rollback", cwd=repo)
    assert out.returncode != 0, out.stdout + out.stderr
    assert "cannot restore" in out.stderr and "id.toml" in out.stderr
    assert selected(sandbox) == 2
    assert dest.read_bytes() == second
    assert not (state(sandbox) / "journal").exists() or not list(
        (state(sandbox) / "journal").glob("*.json")
    )


def legacy_template_target(sandbox: Path) -> tuple[Path, Path, bytes]:
    """Generation 1 as an older `template` receipt: the raw template (the
    `source` the store keeps beside `output`), recorded variables, and the
    rendered bytes' identity."""
    repo, configs = template_repo(sandbox)
    apply(repo)
    dest = sandbox / ".config/app/id.toml"
    rendered = dest.read_bytes()
    current = entry(sandbox, 1, "~/.config/app/id.toml")
    source = current["from"].rsplit("/", 1)[0] + "/source"
    rewrite_entry(
        sandbox, 1, "~/.config/app/id.toml",
        **{"from": source, "mode": "template", "vars": {"email": "a@b.c"},
           "hash": bytes_hash(rendered), "file_mode": 0o644, "source_executable": None},
    )
    (configs / "id.toml").write_text('email = "{{ email }}"\nversion = 2\n')
    apply(repo)
    return repo, dest, rendered


def test_rollback_restores_an_older_template_receipt_exactly(sandbox):
    """An older `template` receipt re-renders from the target's retained
    source with its recorded variables, and the result is checked against the
    recorded identity before it is installed."""
    repo, dest, rendered = legacy_template_target(sandbox)
    assert dest.read_bytes() != rendered
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_bytes() == rendered
    assert dest.stat().st_mode & 0o7777 == 0o644
    assert selected(sandbox) == 1


def test_rollback_refuses_an_older_template_receipt_that_no_longer_renders_its_record(sandbox):
    """R1's real residue: re-rendering is checked against nothing. Recorded
    variables that do not reproduce the recorded identity used to deploy the
    wrong bytes and flip; now rollback refuses before any write."""
    repo, dest, rendered = legacy_template_target(sandbox)
    rewrite_entry(sandbox, 1, "~/.config/app/id.toml", vars={"email": "someone-else"})
    second = dest.read_bytes()
    out = grip("rollback", cwd=repo)
    assert out.returncode != 0, out.stdout + out.stderr
    assert "cannot restore" in out.stderr
    assert dest.read_bytes() == second
    assert selected(sandbox) == 2


def test_rollback_refuses_when_a_managed_block_cannot_be_restored(sandbox):
    """R3: "no safe restore plan" was a warning after which the selection
    still flipped, and the target manifest claimed a block nothing restored.
    Now: refused, the current generation stays selected, the file untouched."""
    configs = sandbox / "myenv" / "configs"
    configs.mkdir(parents=True)
    (configs / "block.sh").write_text("export A=1\n")
    (configs / "tool.conf").write_text("one\n")
    both = """
import { file, identity, managedBlock, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("app", { files: [
    file({ source: repoFile("configs/tool.conf"), content: identity(), destination: trackedCopyTo("~/.config/app/tool.conf") }),
    file({ source: repoFile("configs/block.sh"), content: identity(), destination: managedBlock("~/.bashrc", "app") })
  ] });
"""
    repo = make_workspace_repo(sandbox / "myenv", both)
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("# mine\n")
    apply(repo)
    assert "export A=1" in bashrc.read_text()
    make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("app", { files: [
    file({ source: repoFile("configs/tool.conf"), content: identity(), destination: trackedCopyTo("~/.config/app/tool.conf") })
  ] });
""",
    )
    (configs / "tool.conf").write_text("two\n")
    apply(repo)
    assert "export A=1" not in bashrc.read_text()
    # the user's file is no longer text a block can be spliced into
    foreign = b"\xff\xfe binary rc\n"
    bashrc.write_bytes(foreign)

    out = grip("rollback", cwd=repo)
    assert out.returncode != 0, out.stdout + out.stderr
    assert "cannot restore" in out.stderr and ".bashrc" in out.stderr
    assert selected(sandbox) == 2
    assert bashrc.read_bytes() == foreign
    # the earlier unit's restore was compensated: nothing half-rolled-back
    assert (sandbox / ".config/app/tool.conf").read_text() == "two\n"


def test_rollback_never_writes_into_the_target_generation(sandbox):
    """FND-2: rollback used to backfill `generations/<n>/env/profile.sh` into
    the existing target generation and delete the pre-0.22 `env/profile.sh`
    outside the journal. A generation without its profile cannot be restored
    exactly: refused, the generation and the legacy file untouched."""
    configs = sandbox / "myenv" / "configs"
    configs.mkdir(parents=True)
    (configs / "a.toml").write_text("one\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ environment, file, identity, lit, profile, repoFile, trackedCopyTo }} from "@gripsack/core";

export default [environment("demo-environment", {{ packages: [], target: {native_target()}, env: {{ EDITOR: lit("demo") }} }}),
profile("demo", {{ files: [
    file({{ source: repoFile("configs/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") }})
  ], environment: "demo-environment" }})];
""",
    )
    apply(repo)
    (configs / "a.toml").write_text("two\n")
    apply(repo)
    home = state(sandbox)
    # generation 1 as pre-0.22 history: no in-generation profile
    (home / "generations/1/env/profile.sh").unlink()
    (home / "generations/1/env").rmdir()
    legacy = home / "env/profile.sh"
    legacy.parent.mkdir()
    legacy.write_text("export EDITOR=legacy\n")
    before = tree_digest(home / "generations/1")

    out = grip("rollback", "1", cwd=repo)
    assert out.returncode != 0, out.stdout + out.stderr
    assert "profile" in out.stderr
    assert tree_digest(home / "generations/1") == before
    assert not (home / "generations/1/env").exists()
    assert legacy.read_text() == "export EDITOR=legacy\n"
    assert selected(sandbox) == 2
    assert (sandbox / ".config/demo/a.toml").read_text() == "two\n"


def test_rollback_reselects_with_a_fresh_transaction(sandbox):
    """K6: rollback publishes no generation; it reselects the target under a
    transaction identity no earlier selection used."""
    repo, configs = template_repo(sandbox)
    apply(repo)
    (configs / "id.toml").write_text('email = "{{ email }}"\nversion = 2\n')
    apply(repo)
    selections = state(sandbox) / "generations/.selections"
    before = set(os.listdir(selections))
    generations = sorted(p.name for p in (state(sandbox) / "generations").iterdir())
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    after = set(os.listdir(selections))
    assert len(after - before) == 1
    assert sorted(p.name for p in (state(sandbox) / "generations").iterdir()) == generations
    current = os.readlink(state(sandbox) / "current")
    assert (after - before).pop() in current
