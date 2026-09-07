"""0041: every reachable recorded mutation/durability cut, not sampled phases.

Real debug binary, real filesystem, isolated snapshot reset. Abrupt process loss
and injected IO errors exercise recovery. The separate Rust trace model proves
ordering under fsync assumptions; SIGKILL is NOT a physical power-loss simulator.
"""
import json
import os
import signal
import shutil
import stat
import subprocess
from pathlib import Path

import pytest
from conftest import GRIP, grip, make_env_repo


def snapshot(home):
    path = home / '.matrix'
    data = path.read_bytes() if path.exists() else None
    mode = stat.S_IMODE(path.stat().st_mode) if path.exists() else None
    current = home / '.local/share/gripsack/current'
    generation = int(current.readlink().name) if current.is_symlink() else None
    hooks = {p.name for p in home.glob('hook-*')}
    return data, mode, generation, hooks


def module(version, installed):
    config = '{ payload: trackedCopy("~/.matrix") }' if installed else '{}'
    return f'''import {{ module, trackedCopy, customHook }} from '@gripsack/core';
export default module('demo', {{ config: {config}, env: {{ MATRIX: '{version}' }},
  activate: [customHook('touch "$HOME/hook-{version}"')] }});'''


def command(repo, args, env):
    return subprocess.run([str(GRIP), *args], cwd=repo, env=env, capture_output=True, text=True, timeout=120)


@pytest.mark.parametrize('scenario', ['apply-deploy', 'apply-prune', 'rollback-deploy', 'rollback-prune', 'apply-deploy-copy', 'apply-prune-copy'])
def test_every_reachable_persistence_boundary(sandbox, scenario):
    home = sandbox / 'home'
    home.mkdir()
    env = dict(os.environ, HOME=str(home), GRIPSACK_HOME=str(home / '.local/share/gripsack'))
    (home / '.matrix').write_text('origin')
    (home / '.matrix').chmod(0o600)
    repo = make_env_repo(sandbox / 'env', module('one', scenario != 'rollback-prune'))
    (repo / 'payload').write_text('one')
    base = ['apply', '--host', 'testhost', '--jobs', '1', '--take-over']
    out = command(repo, base, env)
    assert out.returncode == 0, out.stderr
    if scenario.startswith('rollback'):
        (repo / 'modules/hello.ts').write_text(module('two', True))
        (repo / 'payload').write_text('two')
        out = command(repo, base, env)
        assert out.returncode == 0, out.stderr
        operation = ['rollback', '1']
    else:
        (repo / 'modules/hello.ts').write_text(module('two', 'deploy' in scenario))
        (repo / 'payload').write_text('two')
        operation = ['apply', '--host', 'testhost', '--jobs', '1']
    # Hook effects are idempotent markers; only this transition may create them.
    for marker in home.glob('hook-*'):
        marker.unlink()
    old = snapshot(home)
    saved = sandbox / 'saved'
    shutil.copytree(home, saved, symlinks=True)
    trace = sandbox / 'trace.tsv'
    active = dict(env, GRIPSACK_FS_TRACE=str(trace))
    if scenario.endswith('copy'):
        active['GRIPSACK_FS_FORCE_COPY'] = '1'
    out = command(repo, operation, active)
    assert out.returncode == 0, out.stderr
    target = snapshot(home)
    points = [line.split('\t', 3) for line in trace.read_text().splitlines()]
    assert points, 'debug binary did not emit persistence boundaries'
    assert target[:2] != old[:2], out.stdout + out.stderr + (home / '.local/share/gripsack/current/manifest.json').read_text()
    assert target[1] == 0o600, 'the transition widened private-file permissions'
    assert any(row[2] == 'FilePublish' for row in points)
    assert any('activation.json' in row[3] for row in points)
    assert any('current' in row[3] for row in points)

    for fault in ['error', 'kill']:
        for cut, expected in enumerate(points, 1):
            for drift in [False, True]:
                shutil.rmtree(home)
                shutil.copytree(saved, home, symlinks=True)
                trace.unlink(missing_ok=True)
                injected = dict(active, GRIPSACK_FS_CUT=str(cut), GRIPSACK_FS_FAULT=fault)
                result = command(repo, operation, injected)
                emitted = [line.split('\t', 3) for line in trace.read_text().splitlines()]
                context = f'{scenario} {fault} cut={cut}/{len(points)} {expected[1:3]} drift={drift}'
                assert len(emitted) >= cut, context + '\n' + result.stderr
                assert emitted[cut - 1][1:3] == expected[1:3], context + ': boundary sequence changed'
                if fault == 'kill':
                    assert result.returncode == -signal.SIGKILL, context + ': process was not killed'
                if drift:
                    (home / '.matrix').write_text('user-change')
                    (home / '.matrix').chmod(0o640)
                # Recovery only executes the shipped reconcile/resume methods.
                recovery = command(repo, ['apply', '--host', 'testhost'], dict(env, GRIPSACK_FS_RECOVER_ONLY='1'))
                assert recovery.returncode == 0, context + '\n' + recovery.stderr
                actual = snapshot(home)
                assert actual[2] in (old[2], target[2]), context
                expected_state = target if actual[2] == target[2] else old
                if drift:
                    assert actual[:2] == (b'user-change', 0o640), context
                else:
                    assert actual[:2] == expected_state[:2], context + repr((actual, expected_state))
                assert actual[3] == expected_state[3], context + ': activation not resumed/discarded correctly'
                # A second recovery is idempotent and consumes no new generation.
                again = command(repo, ['apply', '--host', 'testhost'], dict(env, GRIPSACK_FS_RECOVER_ONLY='1'))
                assert again.returncode == 0, context + '\n' + again.stderr
                assert snapshot(home) == actual, context + ': recovery was not idempotent'
    print(f'{scenario}: {len(points)} boundaries x 2 failure kinds x 2 drift states verified')
