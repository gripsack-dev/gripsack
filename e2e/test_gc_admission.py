"""0041: metadata/config admission must precede any destructive GC work."""
import errno
import json
import os
import shutil
import signal
import subprocess
from pathlib import Path
import pytest
from conftest import GRIP, grip, make_env_repo


def setup(sandbox):
    repo = make_env_repo(sandbox / 'env', '''import { module, symlink } from '@gripsack/core';
export default module('demo', { install: { payload: symlink('~/.owned') } });''')
    for content in ['one', 'two']:
        (repo / 'payload').write_text(content)
        out = grip('apply', '--host', 'testhost', cwd=repo)
        assert out.returncode == 0, out.stderr
    home = sandbox / '.local/share/gripsack'
    return repo, home


def test_prior_directory_symlink_blocks_gc_before_any_deletion(sandbox):
    repo, home = setup(sandbox)
    outside = sandbox / "outside-priors"
    outside.mkdir()
    sentinel = outside / ("ab" * 32)
    sentinel.write_bytes(b"not owned by the prior store")
    (home / "prior").symlink_to(outside, target_is_directory=True)
    orphan = home / "store" / "orphan-candidate"
    orphan.mkdir()
    (orphan / "content").write_text("unreferenced")
    before = {entry.name for entry in (home / "store").iterdir()}
    for args in (["gc", "--dry-run"], ["gc"]):
        result = grip(*args, cwd=repo)
        assert result.returncode != 0
        assert sentinel.read_bytes() == b"not owned by the prior store"
        assert {entry.name for entry in (home / "store").iterdir()} == before
        assert (home / "generations/1").is_dir()


@pytest.mark.parametrize('bad', ['root', 'nested', 'traversal', 'relative', 'outside'])
@pytest.mark.parametrize('field', ['store_path', 'build_closure'])
def test_invalid_roots_block_collection_without_deleting(sandbox, bad, field):
    repo, home = setup(sandbox)
    path = home / 'current/manifest.json'
    data = json.loads(path.read_text())
    root = Path(data['modules']['demo']['store_path'])
    value = {'root': str(home / 'store'), 'nested': str(root / 'nested'),
             'traversal': str(home / 'store/../bad'), 'relative': 'store/object',
             'outside': str(sandbox / 'outside')}[bad]
    data['modules']['demo'][field] = [value] if field == 'build_closure' else value
    path.write_text(json.dumps(data))
    before = {p.name for p in (home / 'store').iterdir()}
    out = grip('gc', cwd=repo)
    assert out.returncode != 0
    assert {p.name for p in (home / 'store').iterdir()} == before
    assert (home / 'generations/1').exists() and root.exists()
    assert (sandbox / '.owned').read_text() == 'two'


@pytest.mark.parametrize('layer', ['repo', 'user', 'directory', 'broken-user-link'])
def test_invalid_retention_policy_cannot_fall_back(sandbox, layer):
    repo, home = setup(sandbox)
    user = sandbox / '.config/gripsack'
    user.mkdir(parents=True)
    (user / 'config.toml').write_text('[settings]\nkeep_generations = 1\n')
    if layer == 'repo':
        (repo / 'env.toml').write_text('[env]\nname="fixture"\n[settings]\nkeep_generations="many"\n')
    elif layer == 'user':
        (user / 'config.toml').write_text('[settings]\nkeep_generations="many"\n')
    else:
        (user / 'config.toml').unlink()
        if layer == 'directory':
            (user / 'config.toml').mkdir()
        else:
            (user / 'config.toml').symlink_to('missing.toml')
    out = grip('gc', cwd=repo)
    assert out.returncode != 0 and 'E400' in out.stderr
    assert (home / 'generations/1').exists()
    assert (sandbox / '.owned').read_text() == 'two'


def test_valid_repo_precedence_and_missing_artifact_references(sandbox):
    repo, home = setup(sandbox)
    user = sandbox / '.config/gripsack'
    user.mkdir(parents=True)
    (user / 'config.toml').write_text('[settings]\nkeep_generations = 1\n')
    (repo / 'env.toml').write_text('[env]\nname="fixture"\n[settings]\nkeep_generations=2\n')
    # A well-shaped missing closure is allowed (the store repair contract).
    manifest = home / 'current/manifest.json'
    data = json.loads(manifest.read_text())
    data['modules']['demo']['build_closure'] = [str(home / 'store/missing-repair-artifact')]
    manifest.write_text(json.dumps(data))
    out = grip('gc', cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (home / 'generations/1').exists()
    (repo / 'env.toml').write_text('[env]\nname="fixture"\n')
    out = grip('gc', cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not (home / 'generations/1').exists()


def test_unfinished_recovery_blocks_gc_then_recovers(sandbox):
    """0045 F3: a crash window leaves a journaled prior blob that no
    retained manifest references. GC must refuse (nothing deleted,
    dry-run included); after the next apply reconciles, GC passes and
    the recovered bytes are intact."""
    import hashlib

    repo, home = setup(sandbox)
    # This hand-written historical marker must be paired with a historical
    # pointer, not a new transaction selection its writer never emitted.
    (home / "current").unlink()
    (home / "current").symlink_to("generations/2")
    # crash window: run marker + one journaled entry + its prior blob
    journal = home / 'journal'
    journal.mkdir(exist_ok=True)
    (journal / 'run.json').write_text(
        '{"previous_generation": 2, "target_generation": 3, "op": "apply"}')
    blob_bytes = b'user bytes only the journal references\n'
    blob = hashlib.sha256(blob_bytes).hexdigest()
    (home / 'prior').mkdir(exist_ok=True)
    (home / 'prior' / blob).write_bytes(blob_bytes)
    dest = str(sandbox / '.owned')
    entry = {
        'v': 1,
        'dest': dest,
        'prior': {'kind': 'file', 'hash': blob, 'mode': 420},
        'after': {'kind': 'removed'},
    }
    (journal / (hashlib.sha256(dest.encode()).hexdigest() + '.json')).write_text(
        json.dumps(entry))
    store_before = {p.name for p in (home / 'store').iterdir()}

    for args in (['gc'], ['gc', '--dry-run']):
        out = grip(*args, cwd=repo)
        assert out.returncode != 0, f'{args}: must refuse while recovery is pending'
        assert {p.name for p in (home / 'store').iterdir()} == store_before
        assert (home / 'prior' / blob).exists(), 'recovery evidence is never collected'
        assert (home / 'generations/2').exists()

    # Recovery preserves the unrelated live link and drains the old entry.
    # Its orphaned prior is collectable only after that recovery completes.
    out = grip('apply', '--host', 'testhost', cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not list(journal.glob('*.json')), 'journal drained'
    assert (sandbox / '.owned').read_text() == 'two'

    out = grip('gc', cwd=repo)
    assert out.returncode == 0, out.stderr
    # the orphaned blob is collectable only NOW that recovery is done
    assert not (home / 'prior' / blob).exists()


@pytest.mark.parametrize("root_name", ["store", "generations", "prior"])
@pytest.mark.parametrize("dry_run", [True, False])
def test_substituted_collection_root_preserves_foreign_bytes_and_history(
    sandbox, root_name, dry_run
):
    repo, home = setup(sandbox)
    (repo / "env.toml").write_text("[env]\nname='fixture'\n[settings]\nkeep_generations=1\n")
    root = home / root_name
    root.mkdir(exist_ok=True)
    original = home / (root_name + "-retained")
    manifests = [(home / f"generations/{n}/manifest.json").read_bytes() for n in (1, 2)]
    store_names = {entry.name for entry in (home / "store").iterdir()}
    root.rename(original)
    outside = sandbox / "foreign"
    (outside / "foreign-directory").mkdir(parents=True)
    sentinel = outside / "foreign-directory/sentinel"
    sentinel.write_bytes(b"unowned fixture bytes")
    root.symlink_to(outside, target_is_directory=True)
    out = grip("gc", *(["--dry-run"] if dry_run else []), cwd=repo)
    assert out.returncode != 0, out.stdout
    assert sentinel.read_bytes() == b"unowned fixture bytes"
    root.unlink()
    original.rename(root)
    assert [(home / f"generations/{n}/manifest.json").read_bytes() for n in (1, 2)] == manifests
    assert {entry.name for entry in (home / "store").iterdir()} == store_names


@pytest.mark.parametrize("shape", ["file", "broken-link", "non-utf8-child"])
@pytest.mark.parametrize("dry_run", [True, False])
def test_invalid_store_inventory_precedes_generation_pruning(sandbox, shape, dry_run):
    import os

    repo, home = setup(sandbox)
    (repo / "env.toml").write_text("[env]\nname='fixture'\n[settings]\nkeep_generations=1\n")
    before = [(home / f"generations/{n}/manifest.json").read_bytes() for n in (1, 2)]
    store = home / "store"
    if shape == "non-utf8-child":
        try:
            os.mkdir(os.fsencode(store) + b"/invalid-\xff")
        except OSError as error:
            if error.errno == errno.EILSEQ:
                pytest.skip("this filesystem rejects non-UTF-8 names before GC admission")
            raise
    else:
        store.rename(home / "retained-store")
        if shape == "file":
            store.write_bytes(b"not a collection directory")
        else:
            store.symlink_to(sandbox / "absent")
    out = grip("gc", *(["--dry-run"] if dry_run else []), cwd=repo)
    assert out.returncode != 0, out.stdout
    assert [(home / f"generations/{n}/manifest.json").read_bytes() for n in (1, 2)] == before


def test_collecting_orphan_symlink_never_traverses_its_payload(sandbox):
    repo, home = setup(sandbox)
    outside = sandbox / "foreign"
    outside.mkdir()
    sentinel = outside / "sentinel"
    sentinel.write_bytes(b"foreign payload")
    orphan = home / "store/orphan-link"
    orphan.symlink_to(outside, target_is_directory=True)
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not orphan.is_symlink()
    assert sentinel.read_bytes() == b"foreign payload"
    assert (sandbox / ".owned").read_text() == "two"


@pytest.mark.parametrize("fault", ["error", "kill"])
def test_generation_prune_sync_failure_preserves_payloads_on_retry(sandbox, fault):
    repo, home = setup(sandbox)
    (repo / "env.toml").write_text('[env]\nname="fixture"\n[settings]\nkeep_generations=1\n')
    previous = (home / "current").readlink()
    generation = home / "generations/1"
    manifest = json.loads((generation / "manifest.json").read_text())
    payload = Path(manifest["modules"]["demo"]["store_path"])
    saved_generation = sandbox / "saved-generation"
    saved_payload = sandbox / "saved-payload"
    shutil.copytree(generation, saved_generation, symlinks=True)
    shutil.copytree(payload, saved_payload, symlinks=True)
    trace = sandbox / "gc-prune-boundaries.tsv"

    def collect(extra=None):
        return subprocess.run(
            [str(GRIP), "gc"], cwd=repo, env={**os.environ, **(extra or {})},
            capture_output=True, text=True, timeout=30,
        )

    def barrier_cut():
        trace.unlink(missing_ok=True)
        observed = collect({"GRIPSACK_FS_TRACE": str(trace)})
        assert observed.returncode == 0, observed.stdout + observed.stderr
        assert not generation.exists() and not payload.exists()
        rows = [line.split("\t", 3) for line in trace.read_text().splitlines()]
        barriers = [int(row[0]) for row in rows
                    if row[1:] == ["Before", "DirSync", '"generations"']]
        assert barriers, "GC discarded generation roots without durable namespace pruning"
        # Manifest and counter admission may also seal this directory. Cut the
        # final post-prune barrier, not an earlier admission barrier.
        return barriers[-1]

    def refuse(cut):
        failed = collect({"GRIPSACK_FS_CUT": str(cut), "GRIPSACK_FS_FAULT": fault})
        assert failed.returncode != 0, failed.stdout + failed.stderr
        if fault == "kill":
            assert failed.returncode == -signal.SIGKILL
        assert not generation.exists()
        assert (payload / "payload").read_text() == "one"
        assert (home / "current").readlink() == previous
        assert (sandbox / ".owned").read_text() == "two"

    prune_cut = barrier_cut()
    shutil.copytree(saved_generation, generation, symlinks=True)
    shutil.copytree(saved_payload, payload, symlinks=True)
    refuse(prune_cut)
    # The removed generation is already invisible on retry. An empty prune
    # list must still seal that observation before its payload can be collected.
    empty_cut = barrier_cut()
    shutil.copytree(saved_payload, payload, symlinks=True)
    refuse(empty_cut)
    completed = collect()
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert not payload.exists()
    assert (sandbox / ".owned").read_text() == "two"


def legacy_allocation_history(sandbox, high_water):
    repo = make_env_repo(sandbox / "legacy-env", '''
import { module, symlink } from "@gripsack/core";
export default module("demo", {config: {source: symlink("~/.legacy-allocation")}});
''')
    (repo / "source").write_text("new generation\n")
    (repo / "env.toml").write_text('[env]\nname="fixture"\n[settings]\nkeep_generations=0\n')
    home = sandbox / ".local/share/gripsack"
    for number in (10, 20):
        generation = home / "generations" / str(number)
        generation.mkdir(parents=True)
        (generation / "manifest.json").write_text(json.dumps({"number": number, "modules": {}}))
    (home / "current").symlink_to("generations/10")
    if high_water is not None:
        (home / "generations/high-water").write_text(str(high_water))
    return repo, home


@pytest.mark.parametrize("high_water", [None, 12, 30, 2**64 - 1])
def test_gc_preserves_legacy_allocation_history(sandbox, high_water):
    repo, home = legacy_allocation_history(sandbox, high_water)
    collected = grip("gc", cwd=repo)
    assert collected.returncode == 0, collected.stdout + collected.stderr
    assert not (home / "generations/20").exists()
    assert (home / "current").readlink() == Path("generations/10")
    floor = max(20, high_water or 0)
    assert int((home / "generations/high-water").read_text()) == floor
    applied = grip("apply", "--host", "testhost", cwd=repo)
    destination = sandbox / ".legacy-allocation"
    if floor == 2**64 - 1:
        assert applied.returncode != 0
        assert (home / "current").readlink() == Path("generations/10")
        assert not destination.exists() and not destination.is_symlink()
    else:
        assert applied.returncode == 0, applied.stdout + applied.stderr
        assert (home / "current").resolve().name == str(floor + 1)
        assert destination.read_text() == "new generation\n"


@pytest.mark.parametrize("high_water", [None, 30])
def test_allocation_floor_sync_failure_prevents_pruning(sandbox, high_water):
    repo, home = legacy_allocation_history(sandbox, high_water)
    generation = home / "generations/20"
    manifest = (generation / "manifest.json").read_bytes()
    orphan = home / "store/unreferenced"
    trace = sandbox / "allocation-floor-boundaries.tsv"
    counter = home / "generations/high-water"

    def seed():
        generation.mkdir(exist_ok=True)
        (generation / "manifest.json").write_bytes(manifest)
        if high_water is None:
            counter.unlink(missing_ok=True)
        else:
            counter.write_text(str(high_water))
        orphan.mkdir(parents=True, exist_ok=True)
        (orphan / "data").write_text("collect only after preserving allocation history\n")

    def collect(extra=None):
        return subprocess.run(
            [str(GRIP), "gc"], cwd=repo, env={**os.environ, **(extra or {})},
            capture_output=True, text=True, timeout=30,
        )

    seed()
    observed = collect({"GRIPSACK_FS_TRACE": str(trace)})
    assert observed.returncode == 0, observed.stdout + observed.stderr
    assert not generation.exists() and not orphan.exists()
    rows = [line.split("\t", 3) for line in trace.read_text().splitlines()]
    pruning = next(i for i, row in enumerate(rows)
                   if row[1:] == ["Before", "Unlink", '"20"'])
    files = [i for i, row in enumerate(rows[:pruning])
             if row[1:] == ["Before", "FileSync", '"high-water"']]
    assert files, "generation pruning discarded allocation history without a durable counter"
    file_seal = files[-1]
    parent_seal = next(i for i in range(file_seal + 1, pruning)
                       if rows[i][1:3] == ["Before", "DirSync"])
    for cut in [int(rows[file_seal][0]), int(rows[parent_seal][0])]:
        for fault in ["error", "kill"]:
            seed()
            refused = collect({"GRIPSACK_FS_CUT": str(cut), "GRIPSACK_FS_FAULT": fault})
            assert refused.returncode != 0, refused.stdout + refused.stderr
            if fault == "kill":
                assert refused.returncode == -signal.SIGKILL
            assert (generation / "manifest.json").read_bytes() == manifest
            assert (orphan / "data").read_text() == "collect only after preserving allocation history\n"
            assert (home / "current").readlink() == Path("generations/10")
            completed = collect()
            assert completed.returncode == 0, completed.stdout + completed.stderr
            assert not generation.exists() and not orphan.exists()
            assert int(counter.read_text()) == max(20, high_water or 0)
