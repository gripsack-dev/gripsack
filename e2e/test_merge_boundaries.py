"""0044: ambiguous markers cannot consume unowned text or grant authority."""
import json

import pytest
from conftest import grip, make_env_repo, remove_module


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


@pytest.fixture
def merge_repo(sandbox):
    repo = make_env_repo(sandbox / 'env', '''import { module, merge } from '@gripsack/core';
export default module('shell', { config: { payload: merge('~/.bashrc') } });''')
    (repo / 'payload').write_text('managed-one\n')
    (sandbox / '.bashrc').write_bytes(b'USER-PREFIX\r\n')
    run(repo, 'apply', '--host', 'testhost')
    return repo


def managed_block(destination):
    content = destination.read_bytes()
    return content[content.index(b'# >>>'):]


@pytest.mark.parametrize('shape', ['unclosed', 'nested', 'interleaved', 'orphan-close'])
def test_ambiguous_markers_abort_without_losing_foreign_tail(sandbox, merge_repo, shape):
    repo = merge_repo
    dest = sandbox / '.bashrc'
    block = managed_block(dest)
    opener = block.splitlines(keepends=True)[0]
    if shape == 'unclosed':
        broken = block + opener
    elif shape == 'nested':
        broken = opener + block
    elif shape == 'interleaved':
        broken = opener + b'# <<< gripsack module=other <<<\n'
    else:
        broken = block + b'# <<< gripsack module=shell <<<\n'
    before = b'USER-PREFIX\r\n' + broken + b'USER-OWNED-TAIL\n\n'
    dest.write_bytes(before)
    (repo / 'payload').write_text('managed-two\n')
    pointer = (sandbox / '.local/share/gripsack/current').readlink()
    for command in ['plan', 'apply']:
        result = grip(command, '--host', 'testhost', cwd=repo)
        assert result.returncode != 0, result.stdout + result.stderr
        assert dest.read_bytes() == before
        assert (sandbox / '.local/share/gripsack/current').readlink() == pointer
    remove_module(repo, 'hello')
    result = grip('apply', '--host', 'testhost', cwd=repo)
    assert result.returncode != 0
    assert dest.read_bytes() == before
    assert (sandbox / '.local/share/gripsack/current').readlink() == pointer


@pytest.mark.parametrize('conflict_first', [False, True])
def test_every_duplicate_mode_record_guards_reapply_and_prune(sandbox, merge_repo, conflict_first):
    repo = merge_repo
    dest = sandbox / '.bashrc'
    block = managed_block(dest)
    conflict = block.replace(b'mode=0644', b'mode=0600')
    before = b'USER-PREFIX\r\n' + (conflict + block if conflict_first else block + conflict) + b'USER-TAIL\n'
    dest.write_bytes(before)
    for _ in range(2):
        run(repo, 'apply', '--host', 'testhost')
        assert dest.read_bytes() == before
        manifest = json.loads((sandbox / '.local/share/gripsack/current/manifest.json').read_text())
        assert manifest['modules']['shell']['entries'][0]['preserved_drift']
    remove_module(repo, 'hello')
    run(repo, 'apply', '--host', 'testhost')
    assert dest.read_bytes() == before


def test_later_edits_are_reported_and_foreign_bytes_survive(sandbox, merge_repo):
    repo = merge_repo
    dest = sandbox / '.bashrc'
    block = managed_block(dest)
    between = b'USER-MIDDLE\n\r\n'
    tail = b'USER-TAIL\r\n\n'
    dest.write_bytes(b'USER-PREFIX\r\n' + block + between + block.replace(b'managed-one', b'edited-second') + tail)
    result = run(repo, 'apply', '--host', 'testhost')
    assert 'hand-edited' in result.stdout
    assert b'edited-second' not in dest.read_bytes()
    remove_module(repo, 'hello')
    run(repo, 'apply', '--host', 'testhost')
    assert dest.read_bytes() == b'USER-PREFIX\r\n' + between + tail
