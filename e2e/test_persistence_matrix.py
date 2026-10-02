"""0041: every reachable recorded mutation/durability cut, not sampled phases.

Real debug binary, real filesystem, isolated snapshot reset. Abrupt process loss
and injected IO errors exercise recovery. The separate Rust trace model proves
ordering under fsync assumptions; SIGKILL is NOT a physical power-loss simulator.
"""
import json
import os
import signal
import shutil
import platform
import stat
import time
from pathlib import Path

import pytest
from conftest import GRIP, grip, make_env_repo, run_grip


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
    return run_grip([str(GRIP), *args], cwd=repo, env=env, capture_output=True, text=True, timeout=120)


# Zero-based round-robin partitions retain every traced boundary, including
# newly added ones. Both drift states stay together; no configuration means
# the complete local matrix. Never allow an empty required partition.
def cut_partition(total, environment):
    index = environment.get('GRIPSACK_PERSISTENCE_SHARD')
    count = environment.get('GRIPSACK_PERSISTENCE_SHARDS')
    if index is None and count is None:
        return 0, 1, range(1, total + 1)
    if index is None or count is None or not index.isdecimal() or not count.isdecimal():
        raise ValueError('persistence shard index/count must both be nonnegative decimal integers')
    index, count = int(index), int(count)
    if not 0 <= index < count <= total:
        raise ValueError(f'invalid or vacuous persistence partition: {index}/{count}, cuts={total}')
    return index, count, range(index + 1, total + 1, count)


@pytest.mark.parametrize('fault', ['error', 'kill'])
@pytest.mark.parametrize('scenario', ['apply-deploy', 'apply-prune', 'rollback-deploy', 'rollback-prune', 'apply-deploy-copy', 'apply-prune-copy'])
def test_every_reachable_persistence_boundary(sandbox, scenario, fault):
    started = time.monotonic()
    # Validate paired configuration before any expensive fixture setup.
    cut_partition(2**63 - 1, os.environ)
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

    index, count, cuts = cut_partition(len(points), os.environ)
    print(f'{scenario} {fault}: measured {len(points)} cuts; '
          f'partition {index}/{count} selects {len(cuts)} x 2 drift states', flush=True)
    completed = []
    for cut in cuts:
        expected = points[cut - 1]
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
        completed.append(cut)
        print(f'{scenario} {fault}: cut {cut}/{len(points)} both drift states recovered '
              f'and idempotent ({time.monotonic() - started:.1f}s)', flush=True)
    elapsed = time.monotonic() - started
    report = {
        'scenario': scenario, 'fault': fault, 'shard': index, 'shards': count,
        'inventory': [row[1:3] for row in points], 'completed_cuts': completed,
        'drift_states': [False, True], 'seconds': elapsed,
        'system': platform.system(), 'machine': platform.machine(),
        'source': os.environ.get('GITHUB_SHA'),
    }
    if directory := os.environ.get('GRIPSACK_PERSISTENCE_REPORTS'):
        path = Path(directory)
        path.mkdir(parents=True, exist_ok=True)
        (path / f'{platform.system()}-{scenario}-{fault}-{index}.json').write_text(json.dumps(report))
    print(f'{scenario} {fault}: {len(completed)}/{len(points)} boundaries x 2 drift states '
          f'verified in {elapsed:.1f}s (partition {index}/{count})', flush=True)
