"""Private host-bound review execution through the existing apply transaction."""
import json
import os

import pytest

from conftest import grip


def fixture(sandbox, name="saved-workspace"):
    repo = sandbox / name
    repo.mkdir()
    (repo / "config.txt").write_text("reviewed\n")
    (repo / "gripsack.ts").write_text('''
import {defineWorkspace, workspace, profile, file, repoFile, identity, trackedCopyTo} from "@gripsack/core";
export default defineWorkspace(() => workspace({outputs: [profile("personal", {files: [file({
  source: repoFile("config.txt"), content: identity(),
  destination: trackedCopyTo("~/.reviewed-config")
})]})]}));
''')
    return repo


def save(repo, sandbox, name="review.json"):
    path = sandbox / name
    result = grip("plan", "--save", str(path), cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    assert path.stat().st_mode & 0o777 == 0o600
    return path


def apply_saved(repo, path):
    return grip("apply", "--saved-plan", str(path), cwd=repo, approve=False)


def current(sandbox):
    path = sandbox / ".local/share/gripsack/current"
    return os.readlink(path) if path.is_symlink() else None


def test_unchanged_saved_plan_executes_captured_declaration(sandbox):
    repo = fixture(sandbox)
    path = save(repo, sandbox)
    assert not (sandbox / ".reviewed-config").exists()
    result = apply_saved(repo, path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert (sandbox / ".reviewed-config").read_text() == "reviewed\n"
    assert current(sandbox) is not None
    # A consumed authorization is not a reusable force token: its base changed.
    generation = current(sandbox)
    again = apply_saved(repo, path)
    assert again.returncode != 0
    assert current(sandbox) == generation


@pytest.mark.parametrize("change", ["source", "lock", "trust", "foreign_destination", "parent_identity", "tamper", "version"])
def test_stale_or_tampered_saved_plan_refuses_before_deployment(sandbox, change):
    repo = fixture(sandbox)
    destination = sandbox / ".reviewed-config"
    path = save(repo, sandbox)
    if change == "source":
        (repo / "config.txt").write_text("NOT REVIEWED\n")
    elif change == "lock":
        (repo / "gripsack.lock").write_text('{"lock_version":3,"hosts":{}}\n')
    elif change == "trust":
        (sandbox / ".local/share/gripsack/trust.toml").unlink()
    elif change == "foreign_destination":
        destination.write_text("foreign\n")
    elif change == "parent_identity":
        # Permission identity of the existing destination parent is bound too.
        sandbox.chmod(0o750)
    else:
        document = json.loads(path.read_text())
        if change == "tamper":
            document["binding"]["ir"] = document["binding"]["ir"].replace("config.txt", "other.txt")
        else:
            document["version"] += 1
        path.write_text(json.dumps(document))
    before = current(sandbox)
    result = apply_saved(repo, path)
    assert result.returncode != 0, result.stdout + result.stderr
    assert current(sandbox) == before
    if change == "foreign_destination":
        assert destination.read_text() == "foreign\n"
    else:
        assert not destination.exists()


def test_saved_plan_rejects_changed_base_generation(sandbox):
    repo = fixture(sandbox)
    path = save(repo, sandbox)
    applied = grip("apply", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    before = current(sandbox)
    result = apply_saved(repo, path)
    assert result.returncode != 0
    assert current(sandbox) == before
    assert (sandbox / ".reviewed-config").read_text() == "reviewed\n"


@pytest.mark.parametrize("change", ["mode", "replacement", "content"])
def test_saved_plan_rejects_destination_observation_drift(sandbox, change):
    repo = fixture(sandbox)
    applied = grip("apply", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    path = save(repo, sandbox)
    destination = sandbox / ".reviewed-config"
    if change == "mode":
        destination.chmod(0o600 if destination.stat().st_mode & 0o777 != 0o600 else 0o640)
    elif change == "replacement":
        destination.rename(sandbox / ".original-config")
        destination.write_text("reviewed\n")
    else:
        destination.write_text("foreign edit\n")
    observed = (destination.read_bytes(), destination.stat().st_mode)
    before = current(sandbox)
    result = apply_saved(repo, path)
    assert result.returncode != 0
    assert current(sandbox) == before
    assert (destination.read_bytes(), destination.stat().st_mode) == observed


def test_saved_plan_refuses_public_file_and_existing_save_target(sandbox):
    repo = fixture(sandbox)
    path = save(repo, sandbox)
    path.chmod(0o644)
    assert apply_saved(repo, path).returncode != 0
    sentinel = sandbox / "sentinel"
    sentinel.write_text("do not overwrite")
    target = sandbox / "symlink-plan.json"
    target.symlink_to(sentinel)
    result = grip("plan", "--save", str(target), cwd=repo)
    assert result.returncode != 0
    assert sentinel.read_text() == "do not overwrite"

def test_saved_plan_reobserves_declared_probe_without_reevaluating_source(sandbox):
    repo = fixture(sandbox)
    observed = sandbox / "probe-input"
    source = repo / "gripsack.ts"
    text = source.read_text().replace(
        "defineWorkspace(() => workspace(",
        f"defineWorkspace((ctx) => {{ ctx.probe.file_exists({json.dumps(str(observed))}); return workspace(",
    )
    source.write_text(text.replace("})]}));", "})]}); });"))
    path = save(repo, sandbox)
    observed.write_text("new host observation")
    result = apply_saved(repo, path)
    assert result.returncode != 0, result.stdout + result.stderr
    assert "probe" in result.stderr
    assert not (sandbox / ".reviewed-config").exists()


def test_private_local_record_rejects_imported_rehashed_envelope(sandbox):
    repo = fixture(sandbox)
    path = save(repo, sandbox)
    # Even a self-consistent envelope cannot confer authority without its exact
    # retained local admission record. Copying private bytes does not mint one.
    state = sandbox / ".local/share/gripsack/saved-plans"
    for record in state.iterdir():
        record.unlink()
    result = apply_saved(repo, path)
    assert result.returncode != 0
    assert not (sandbox / ".reviewed-config").exists()


@pytest.mark.parametrize("name", ['quoted"workspace', "backslash\\workspace"])
def test_saved_plan_round_trips_native_repository_path_characters(sandbox, name):
    repo = fixture(sandbox, name)
    path = save(repo, sandbox)
    envelope = json.loads(path.read_text())
    ir = json.loads(envelope["binding"]["ir"])
    assert ir["workspace"]["outputs"][0]["name"] == "personal"
    result = apply_saved(repo, path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert (sandbox / ".reviewed-config").read_text() == "reviewed\n"


def test_selected_saved_profile_does_not_require_unrelated_imported_package(sandbox):
    repo = fixture(sandbox)
    source_file = repo / "gripsack.ts"
    source_file.write_text(source_file.read_text().replace(
        "import {defineWorkspace,", "import {pkg, provider, source, defineWorkspace,"
    ).replace(
        'outputs: [profile("personal"',
        'outputs: [pkg("unrelated", {producer: provider(source.package({manifest:'
        + json.dumps("a" * 64)
        + '})), commands: {}, target: {os:"linux", arch:"x86_64"}, layout:{kind:"relocatable"}}), profile("personal"'
    ))
    # Unused package policy is not admission authority for this file-only scope.
    sharing = sandbox / ".local/share/gripsack/sharing"
    sharing.mkdir(parents=True, exist_ok=True)
    (sharing / "trust.json").write_text("{invalid unrelated package policy")
    path = sandbox / "selected.json"
    result = grip("plan", "personal", "--save", str(path), cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    result = apply_saved(repo, path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert (sandbox / ".reviewed-config").read_text() == "reviewed\n"
