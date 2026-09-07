"""0041: authoring-style parity, real ordering, output gates and honest preview."""
import json
from pathlib import Path
import pytest
from conftest import grip, make_env_repo, make_toolchain_tarball, refresh_host


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def current(sandbox):
    return json.loads((sandbox / '.local/share/gripsack/current/manifest.json').read_text())


@pytest.mark.parametrize('explicit', [False, True])
def test_module_verification_is_always_preflip(sandbox, explicit):
    shape = 'steps: [shellStep("echo artifact > payload", "produce")],' if explicit else 'build: { kind: "custom_shell", script: "echo artifact > payload" },'
    repo = make_env_repo(sandbox / 'env', f'''import {{ module, shellStep, verifyShell }} from '@gripsack/core';
export default module('demo', {{ {shape} verify: verifyShell('exit 17') }});''')
    run(repo, 'check', '--host', 'testhost')
    result = grip('apply', '--host', 'testhost', cwd=repo)
    assert result.returncode != 0 and 'E302' in result.stderr
    assert not (sandbox / '.local/share/gripsack/current').exists()


@pytest.mark.parametrize('explicit', [False, True])
def test_config_sources_stage_and_lint_in_both_styles(sandbox, explicit):
    shape = 'steps: [configStep({ "config.toml": trackedCopy("~/.config/helix/config.toml") })]' if explicit else 'config: { "config.toml": trackedCopy("~/.config/helix/config.toml") }'
    repo = make_env_repo(sandbox / 'env', f'''import {{ module, configStep, trackedCopy }} from '@gripsack/core';
export default module('helix', {{ lint: 'helix', {shape} }});''')
    source = repo / 'config.toml'
    source.write_text('[editor\n')
    result = grip('check', '--host', 'testhost', cwd=repo)
    assert result.returncode != 0 and 'A00' in result.stderr
    source.write_text('theme = "base16_default"\n')
    run(repo, 'apply', '--host', 'testhost')
    assert (sandbox / '.config/helix/config.toml').read_text() == source.read_text()
    first = current(sandbox)['modules']['helix']['store_path']
    source.write_text('theme = "onedark"\n')
    run(repo, 'apply', '--host', 'testhost')
    assert (sandbox / '.config/helix/config.toml').read_text() == source.read_text()
    assert current(sandbox)['modules']['helix']['store_path'] != first


@pytest.mark.parametrize('kind', ['shell', 'run'])
def test_outputs_are_postconditions_and_recipes_cache(sandbox, kind):
    missing = 'shellStep("true", "produce", { outputs: ["payload"] })' if kind == 'shell' else 'runStep(["true"], "produce", { outputs: ["payload"] })'
    repo = make_env_repo(sandbox / 'env', f'''import {{ module, shellStep, runStep }} from '@gripsack/core';
export default module('demo', {{ steps: [{missing}] }});''')
    result = grip('apply', '--host', 'testhost', cwd=repo)
    assert result.returncode != 0 and 'E301' in result.stderr
    assert not (sandbox / '.local/share/gripsack/current').exists()
    # No outputs is not an implicit always-run switch: this is an artifact recipe.
    (repo / 'modules/hello.ts').write_text('''import { module, shellStep } from '@gripsack/core';
export default module('demo', { steps: [shellStep('echo ran >> "$HOME/count"; echo artifact > payload', 'produce')] });''')
    run(repo, 'apply', '--host', 'testhost')
    number = current(sandbox)['number']
    run(repo, 'apply', '--host', 'testhost')
    assert (sandbox / 'count').read_text().splitlines() == ['ran']
    assert current(sandbox)['number'] == number


@pytest.mark.parametrize('scope', [[], ['a-consumer']])
def test_cross_module_needs_orders_and_scopes_without_installing_tools(sandbox, scope):
    repo = make_env_repo(sandbox / 'env', {
        'a-consumer': '''import { module, shellStep } from '@gripsack/core';
export default module('a-consumer', { steps: [shellStep('test -f "$HOME/ready"', 'consume', { needs: ['z-producer:produce'] })] });''',
        'z-producer': '''import { module, shellStep } from '@gripsack/core';
export default module('z-producer', { steps: [shellStep('touch "$HOME/ready"', 'produce')] });''',
    })
    run(repo, 'apply', *scope, '--host', 'testhost', '--jobs', '1')
    assert (sandbox / 'ready').exists()
    assert set(current(sandbox)['modules']) == {'a-consumer', 'z-producer'}


@pytest.mark.parametrize('need', ['a:produce', 'b:activate', 'b:missing'])
def test_unfulfillable_cross_references_fail_check(sandbox, need):
    repo = make_env_repo(sandbox / 'env', {
        'a': f'''import {{ module, shellStep }} from '@gripsack/core';
export default module('a', {{ steps: [shellStep('true', 'produce', {{ needs: [{json.dumps(need)}] }})] }});''',
        'b': '''import { module, customHook } from '@gripsack/core';
export default module('b', { activate: [customHook('true')] });''',
    })
    result = grip('check', '--host', 'testhost', cwd=repo)
    assert result.returncode != 0 and ('E104' in result.stderr or 'E121' in result.stderr)


def test_mixed_dependency_cycle_fails_before_mutation(sandbox):
    repo = make_env_repo(sandbox / 'env', {
        'a': '''import { module, dep } from '@gripsack/core'; export default module('a', { depends: [dep('b')] });''',
        'b': '''import { module, shellStep } from '@gripsack/core'; export default module('b', { steps: [shellStep('true','produce',{needs:['a:done']})] });''',
    })
    result = grip('check', '--host', 'testhost', cwd=repo)
    assert result.returncode != 0 and 'E120' in result.stderr
    assert not (sandbox / '.local/share/gripsack/current').exists()


def test_warm_multi_entry_preview_uses_actual_store_sources(sandbox):
    payload = make_toolchain_tarball(sandbox / 'tool.tar.gz', {'bin/tool': b'#!/bin/sh\necho ok\n', 'share/doc': b'docs\n'})
    repo = make_env_repo(sandbox / 'env', {
        'fetched': f'''import {{ module, fileFetch, symlink }} from '@gripsack/core'; export default module('fetched', {{ fetch: fileFetch({json.dumps(str(payload))}), install: {{ 'bin/tool': symlink('~/.local/bin/tool'), 'share/doc': symlink('~/.doc') }} }});''',
        'config': '''import { module, symlink } from '@gripsack/core'; export default module('config', {config:{payload:symlink('~/.owned')}});''',
    })
    (repo / 'payload').write_text('local\n')
    run(repo, 'apply', '--host', 'testhost')
    plan = run(repo, 'plan', '--host', 'testhost')
    assert plan.stdout.count('(satisfied)') == 3, plan.stdout
    assert '(new)' not in plan.stdout and '(update)' not in plan.stdout
    number = current(sandbox)['number']
    run(repo, 'apply', '--host', 'testhost')
    assert current(sandbox)['number'] == number


@pytest.mark.parametrize('where', ['module', 'step'])
def test_removed_retries_are_rejected(sandbox, where):
    fields = 'retries: 2' if where == 'module' else 'steps: [shellStep("true", "produce", { retries: 2 } as never)]'
    repo = make_env_repo(sandbox / 'env', f'''import {{ module, shellStep }} from '@gripsack/core'; export default module('demo', {{ {fields} }} as never);''')
    result = grip('check', '--host', 'testhost', cwd=repo)
    assert result.returncode != 0 and 'retries' in result.stderr


def test_private_copy_updates_and_exec_changes_keep_acquired_permissions(sandbox):
    repo = make_env_repo(sandbox / 'env', '''import { module, trackedCopy } from '@gripsack/core';
export default module('demo', { config: { payload: trackedCopy('~/.private') } });''')
    destination = sandbox / '.private'
    destination.write_text('original')
    destination.chmod(0o600)
    source = repo / 'payload'
    source.write_text('one')
    source.chmod(0o644)
    run(repo, 'apply', '--host', 'testhost', '--take-over')
    generation = current(sandbox)['number']
    run(repo, 'apply', '--host', 'testhost')
    assert current(sandbox)['number'] == generation
    source.write_text('two')
    run(repo, 'apply', '--host', 'testhost')
    assert destination.read_text() == 'two'
    assert destination.stat().st_mode & 0o777 == 0o600
    source.chmod(0o755)
    run(repo, 'apply', '--host', 'testhost')
    assert destination.stat().st_mode & 0o777 == 0o700
    source.chmod(0o644)
    run(repo, 'apply', '--host', 'testhost')
    assert destination.stat().st_mode & 0o777 == 0o600
    result = run(repo, 'store-verify')
    assert 'corrupt' not in result.stdout + result.stderr


@pytest.mark.parametrize('next_content', ['one', 'two'])
def test_template_rollback_restores_full_permissions(sandbox, next_content):
    def declaration(revision):
        return f'''import {{ module, template }} from '@gripsack/core';
export default module('demo', {{ config: {{ payload: template('~/.private') }}, env: {{ REVISION: '{revision}' }} }});'''

    repo = make_env_repo(sandbox / 'env', declaration(1))
    destination = sandbox / '.private'
    destination.write_text('original')
    destination.chmod(0o600)
    source = repo / 'payload'
    source.write_text('one')
    run(repo, 'apply', '--host', 'testhost', '--take-over')
    destination.chmod(0o640)
    source.write_text(next_content)
    (repo / 'modules/hello.ts').write_text(declaration(2))
    # Explicitly acquire the changed mode; ordinary apply must preserve chmod
    # drift without giving a later rollback permission to overwrite it.
    run(repo, 'apply', '--host', 'testhost', '--take-over')
    assert current(sandbox)['number'] == 2
    assert destination.stat().st_mode & 0o777 == 0o640
    run(repo, 'rollback', '1')
    assert destination.read_text() == 'one'
    assert destination.stat().st_mode & 0o777 == 0o600


def test_store_repair_preserves_pre_036_private_copy_payloads(sandbox):
    repo = make_env_repo(sandbox / 'env', '''import { module, trackedCopy } from '@gripsack/core';
export default module('demo', { config: { payload: trackedCopy('~/.private') } });''')
    (repo / 'payload').write_text('retained payload')
    run(repo, 'apply', '--host', 'testhost')
    manifest = current(sandbox)
    entry = manifest['modules']['demo']['entries'][0]
    # Pre-0.36 takeover recorded the nominal 0644 hash even when the
    # destination's landed mode was 0600. Reproduce that persisted shape.
    entry['file_mode'] = 0o600
    entry.pop('source_executable', None)
    (sandbox / '.private').chmod(0o600)
    manifest_path = sandbox / '.local/share/gripsack/current/manifest.json'
    manifest_path.write_text(json.dumps(manifest))
    payload = Path(manifest['modules']['demo']['store_path']) / 'payload'
    run(repo, 'store-verify', '--repair')
    assert payload.read_text() == 'retained payload'


def test_store_verify_detects_built_copy_source_exec_tampering(sandbox):
    repo = make_env_repo(sandbox / 'env', '''import { module, shellStep, installStep, trackedCopy } from '@gripsack/core';
export default module('demo', { steps: [
  shellStep('printf payload > payload', 'produce'),
  installStep({ payload: trackedCopy('~/.copy') }, 'install', { needs: ['produce'] }),
] });''')
    run(repo, 'apply', '--host', 'testhost')
    payload = Path(current(sandbox)['modules']['demo']['store_path']) / 'payload'
    payload.chmod(0o555)
    result = grip('store-verify', cwd=repo)
    assert result.returncode != 0, result.stdout + result.stderr
    assert payload.read_text() == 'payload'
