"""0041: metadata/config admission must precede any destructive GC work."""
import json
from pathlib import Path
import pytest
from conftest import grip, make_env_repo


def setup(sandbox):
    repo = make_env_repo(sandbox / 'env', '''import { module, symlink } from '@gripsack/core';
export default module('demo', { install: { payload: symlink('~/.owned') } });''')
    for content in ['one', 'two']:
        (repo / 'payload').write_text(content)
        out = grip('apply', '--host', 'testhost', cwd=repo)
        assert out.returncode == 0, out.stderr
    home = sandbox / '.local/share/gripsack'
    return repo, home


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
