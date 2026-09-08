"""0044: partial failure is observable, and resolved paths agree before publication."""
import io
import json
import tarfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest
from conftest import grip, make_env_repo, make_tarball


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def store_snapshot(sandbox):
    root = sandbox / '.local/share/gripsack/store'
    return {str(path.relative_to(root)): (path.stat().st_mode, path.read_bytes())
            for path in root.rglob('*') if path.is_file() and not path.is_symlink()}


@pytest.mark.parametrize('failed_position', [0, 1, 2])
def test_survey_keeps_all_results_and_error_dominates_changes(sandbox, failed_position):
    names = ['alpha', 'middle', 'omega']
    failed = names.pop(failed_position)
    stable, moving = names
    archive = make_tarball(sandbox / 'stable.tar.gz', {'payload': b'stable'})
    changing = make_tarball(sandbox / 'moving.tar.gz', {'payload': b'one'})
    paths = {failed: sandbox / 'missing.tar.gz', stable: archive, moving: changing}
    repo = make_env_repo(sandbox / 'env', {name: f'''import {{module,fileFetch,trackedCopy}} from '@gripsack/core';
export default module('{name}',{{fetch:fileFetch('{path}'),install:{{payload:trackedCopy('~/.{name}')}}}});'''
        for name, path in paths.items()})
    run(repo, 'update', stable, moving, '--host', 'testhost')
    lock = repo / 'locks/testhost.lock'
    before_lock, before_store = lock.read_bytes(), store_snapshot(sandbox)
    make_tarball(changing, {'payload': b'two'})
    result = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert result.returncode == 2, result.stdout + result.stderr
    lines = result.stdout.splitlines()
    assert any(stable in line and 'unchanged' in line for line in lines)
    assert any(moving in line and 'would bump' in line for line in lines)
    assert any(failed in line and 'failed' in line for line in lines)
    assert 'incomplete' in result.stdout
    assert lock.read_bytes() == before_lock
    assert store_snapshot(sandbox) == before_store
    assert not any((sandbox / f'.{name}').exists() for name in paths)
    changed_only = grip('update', '--check', moving, '--host', 'testhost', cwd=repo)
    assert changed_only.returncode == 1
    run(repo, 'update', '--check', stable, '--host', 'testhost')
    assert grip('update', '--check', 'not-selected', '--host', 'testhost', cwd=repo).returncode == 2
    # Normal update keeps its all-or-nothing lock contract.
    assert grip('update', '--host', 'testhost', cwd=repo).returncode == 1
    assert lock.read_bytes() == before_lock


def test_survey_setup_failure_is_not_an_update_available_exit(sandbox):
    repo = make_env_repo(sandbox / 'env', {})
    run(repo, 'update', '--check', '--host', 'testhost')
    (repo / 'locks').mkdir()
    (repo / 'locks/testhost.lock').write_text('{broken')
    assert grip('update', '--check', '--host', 'testhost', cwd=repo).returncode == 2


class ReleaseServer:
    def __init__(self):
        self.tag = 'v1'
        self.archives = {}
        self.requests = []
        owner = self
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                owner.requests.append(self.path)
                if self.path.startswith('/api/v3/repos/acme/tool/releases'):
                    name = f'pkg-{owner.tag[1:]}.tar.gz'
                    data = json.dumps({'tag_name': owner.tag, 'assets': [{
                        'name': name, 'browser_download_url': f'{owner.base}/downloads/{name}',
                        'url': f'{owner.base}/api/v3/assets/{name}',
                    }]}).encode()
                    content_type = 'application/json'
                else:
                    data = owner.archives[self.path.rsplit('/', 1)[-1]]
                    content_type = 'application/octet-stream'
                self.send_response(200)
                self.send_header('Content-Type', content_type)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            def log_message(self, *args):
                pass
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self):
        return f'http://127.0.0.1:{self.server.server_port}'

    def release(self, tag, marker):
        self.tag = tag
        buffer = io.BytesIO()
        script = f'#!/bin/sh\nprintf {tag} > "{marker}"\n'.encode()
        with tarfile.open(fileobj=buffer, mode='w:gz') as archive:
            info = tarfile.TarInfo(f'pkg-{tag[1:]}/tool')
            info.mode, info.size = 0o755, len(script)
            archive.addfile(info, io.BytesIO(script))
        self.archives[f'pkg-{tag[1:]}.tar.gz'] = buffer.getvalue()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


@pytest.mark.parametrize('explicit', [False, True])
def test_bare_version_preflight_and_historical_paths_agree(sandbox, explicit):
    server = ReleaseServer()
    marker = sandbox / 'verification-ran'
    server.release('v1', marker)
    def module(token):
        source = f"githubRelease({{repo:'acme/tool',asset:'pkg-{{version.bare}}.tar.gz',base_url:'{server.base}'}})"
        path = f'pkg-{{{token}}}/tool'
        if explicit:
            fields = f"steps:[fetchStep({source}),installStep({{'{path}':symlink('~/.tool')}},'install',{{verify:verifyBinary('{path}')}})]"
        else:
            fields = f"fetch:{source},install:{{'{path}':symlink('~/.tool')}},verify:verifyBinary('{path}')"
        return f"import {{module,githubRelease,symlink,verifyBinary,fetchStep,installStep}} from '@gripsack/core'; export default module('tool',{{{fields}}});"
    try:
        repo = make_env_repo(sandbox / 'env', module('version'))
        before = grip('update', '--check', '--host', 'testhost', cwd=repo)
        assert before.returncode == 2, before.stdout + before.stderr
        assert 'preflight' in before.stdout and '{version.bare}' in before.stdout
        assert not (repo / 'locks/testhost.lock').exists()
        assert not store_snapshot(sandbox)
        (repo / 'modules/hello.ts').write_text(module('version.bare'))
        run(repo, 'update', '--host', 'testhost')
        lock = repo / 'locks/testhost.lock'
        assert json.loads(lock.read_text())['modules']['tool']['resolved']['version'] == 'v1'
        assert not marker.exists(), 'layout preflight must not execute verification programs'
        run(repo, 'check', '--host', 'testhost')
        run(repo, 'apply', '--host', 'testhost')
        assert marker.read_text() == 'v1'
        assert 'pkg-1/tool' in str((sandbox / '.tool').readlink())
        server.release('v2', marker)
        run(repo, 'update', '--host', 'testhost')
        run(repo, 'apply', '--host', 'testhost')
        run(repo, 'rollback', '1')
        assert 'pkg-1/tool' in str((sandbox / '.tool').readlink())
        (repo / 'modules/hello.ts').write_text(module('version'))
        # Same fetch pin/cache is available: offline check must catch the real miss.
        request_count = len(server.requests)
        assert grip('check', '--host', 'testhost', cwd=repo).returncode != 0
        assert len(server.requests) == request_count
    finally:
        server.close()


def test_recipe_layout_is_deferred_without_running_the_recipe(sandbox):
    archive = make_tarball(sandbox / 'source.tar.gz', {'source': b'inputs'})
    sentinel = sandbox / 'build-ran'
    repo = make_env_repo(sandbox / 'env', f'''import {{module,fileFetch,symlink}} from '@gripsack/core';
export default module('builder',{{fetch:fileFetch('{archive}'),build:{{kind:'custom_shell',script:'touch {sentinel}; cp source built'}},install:{{built:symlink('~/.built')}}}});''')
    result = run(repo, 'update', '--host', 'testhost')
    assert 'deferred' in result.stdout
    assert not sentinel.exists()
    assert not (sandbox / '.built').exists()
