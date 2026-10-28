"""Workspace deployed identity, installed frontend truth and read-only surveys."""
import json
import subprocess
from pathlib import Path

import pytest
from conftest import grip, make_workspace_repo, make_tarball


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def current(sandbox):
    wire = json.loads((sandbox / '.local/share/gripsack/current/manifest.json').read_text())
    return wire.get("generation", wire)


def template_repo(sandbox):
    repo = make_workspace_repo(sandbox / 'env', '''import {
 profile,file,repoFile,templateFile,trackedCopyTo} from '@gripsack/core';
export default profile('demo', {files:[file({source:repoFile('payload'),
 content:templateFile({value:'rendered'}),destination:trackedCopyTo('~/.wrapper')})]});''')
    (repo / 'payload').write_text('#!/bin/sh\nprintf "{{ value }}\\n"\n')
    (repo / 'payload').chmod(0o755)
    return repo


def test_executable_template_is_identified_and_survives_reapply(sandbox):
    repo = template_repo(sandbox)
    preview = run(repo, 'plan', '--host', 'testhost')
    assert '.wrapper' in preview.stdout
    run(repo, 'apply', '--host', 'testhost')
    dest = sandbox / '.wrapper'
    assert subprocess.check_output([str(dest)], text=True) == 'rendered\n'
    assert dest.read_text().startswith('#!/bin/sh\n'), 'templates must not inject markers'
    first = current(sandbox)['number']
    run(repo, 'apply', '--host', 'testhost')
    assert current(sandbox)['number'] == first
    run(repo, 'store-verify')
    # Source-only executable delta is a real update even when rendered bytes match.
    (repo / 'payload').chmod(0o644)
    run(repo, 'apply', '--host', 'testhost')
    assert dest.stat().st_mode & 0o777 == 0o644
    run(repo, 'rollback', str(first))
    assert subprocess.check_output([str(dest)], text=True) == 'rendered\n'


@pytest.mark.parametrize('reapply', [False, True])
def test_chmod_template_drift_is_visible_and_never_pruned(sandbox, reapply):
    repo = template_repo(sandbox)
    run(repo, 'apply', '--host', 'testhost')
    dest = sandbox / '.wrapper'
    content = dest.read_bytes()
    dest.chmod(0o600)
    preview = run(repo, 'plan', '--host', 'testhost')
    assert 'drift' in preview.stdout.lower()
    # templateFile transforms content; trackedCopyTo still owns a tracked copy,
    # not the removed template ownership mode. Its live drift is reported by
    # plan/apply, while store-verify verifies the immutable rendered artifact.
    run(repo, 'store-verify', '--repair')
    assert dest.read_bytes() == content
    assert dest.stat().st_mode & 0o777 == 0o600
    state = current(sandbox)['modules']['demo']
    payload = Path(state['store_path']) / state['entries'][0]['from']
    assert payload.is_file(), 'destination chmod is not store corruption'
    if reapply:
        for _ in range(2):
            result = run(repo, 'apply', '--host', 'testhost')
            assert 'drift' in (result.stdout + result.stderr).lower()
            assert dest.stat().st_mode & 0o777 == 0o600
        run(repo, 'rollback', '1')
        assert dest.stat().st_mode & 0o777 == 0o600
    (repo / 'outputs/hello.ts').write_text(
        "import {profile} from '@gripsack/core'; export default profile('demo', {files:[]});"
    )
    run(repo, 'apply', '--host', 'testhost')
    assert dest.read_bytes() == content
    assert dest.stat().st_mode & 0o777 == 0o600



@pytest.mark.parametrize('dest_name', ['.bashrc', 'config.html'])
def test_managed_block_chmod_guard(sandbox, dest_name):
    repo = make_workspace_repo(sandbox / 'env', f'''import {{
 profile,file,repoFile,identity,managedBlock}} from '@gripsack/core';
export default profile('demo', {{files:[file({{source:repoFile('payload'),
 content:identity(),destination:managedBlock('~/{dest_name}', 'demo')}})]}});''')
    (repo / 'payload').write_text('managed\n')
    dest = sandbox / dest_name
    dest.write_text('foreign\n')
    dest.chmod(0o600)
    run(repo, 'apply', '--host', 'testhost')
    assert 'mode=0600' in dest.read_text()
    assert current(sandbox)['modules']['demo']['entries'][0]['file_mode'] == 0o600
    dest.chmod(0o644)
    before = dest.read_bytes()
    verify = grip('store-verify', '--repair', cwd=repo)
    assert verify.returncode != 0
    for _ in range(2):
        result = run(repo, 'apply', '--host', 'testhost')
        assert 'drift' in (result.stdout + result.stderr).lower()
        assert dest.read_bytes() == before
    (repo / 'outputs/hello.ts').write_text(
        "import {profile} from '@gripsack/core'; export default profile('demo', {files:[]});"
    )
    run(repo, 'apply', '--host', 'testhost')
    assert dest.read_bytes() == before
    assert dest.stat().st_mode & 0o777 == 0o644


def test_doctor_reports_installed_copy_not_declared_spec(sandbox):
    repo = make_workspace_repo(sandbox / 'env',
        "import {profile} from '@gripsack/core'; export default profile('demo', {files:[]});")
    (repo / 'package.json').write_text(json.dumps({'devDependencies': {'@gripsack/core': '^0.36.0'}}))
    package = repo / 'node_modules/@gripsack/core'
    package.mkdir(parents=True)
    (package / 'package.json').write_text(json.dumps({'version': '0.17.9', 'main': 'index.js'}))
    (package / 'index.js').write_text('export {};\n')
    result = grip('doctor', cwd=repo)
    assert result.returncode != 0
    assert any('MISS' in line and '0.17.9' in line for line in result.stdout.splitlines())
    (package / 'package.json').write_text('{invalid json')
    assert grip('doctor', cwd=repo).returncode != 0
    # No package.json is no shadowing install, exactly as eval resolves it.
    (package / 'package.json').unlink()
    run(repo, 'doctor')


def store_snapshot(sandbox):
    store = sandbox / '.local/share/gripsack/store'
    return {str(p.relative_to(store)): (p.stat().st_mode, p.read_bytes())
            for p in store.rglob('*') if p.is_file() and not p.is_symlink()}


def test_update_check_resolves_without_publishing_or_writing(sandbox):
    archive = make_tarball(sandbox / 'source.tar.gz', {'payload': b'one'})
    repo = make_workspace_repo(sandbox / 'env', f'''import {{
 pkg,provider,tarball}} from '@gripsack/core';
export default pkg('demo', {{producer:provider(tarball({json.dumps(archive.as_uri())})),
 commands:{{}},target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}});''')
    lock = repo / 'gripsack.lock'
    fresh = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert fresh.returncode == 1, fresh.stdout + fresh.stderr
    assert not lock.exists()
    assert not store_snapshot(sandbox)
    run(repo, 'update', '--host', 'testhost')
    before_lock, before_store = lock.read_bytes(), store_snapshot(sandbox)
    run(repo, 'update', '--check', '--host', 'testhost')
    assert lock.read_bytes() == before_lock
    assert store_snapshot(sandbox) == before_store
    make_tarball(archive, {'payload': b'two'})
    bumped = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert bumped.returncode == 1
    assert 'demo' in bumped.stdout
    assert lock.read_bytes() == before_lock
    assert store_snapshot(sandbox) == before_store
    assert not (sandbox / '.copy').exists()
    archive.unlink()
    assert grip('update', '--check', '--host', 'testhost', cwd=repo).returncode != 0
    assert lock.read_bytes() == before_lock
    assert store_snapshot(sandbox) == before_store


