"""0044: real HTTP requests, fake credentials, bounded retries and cold GHE apply."""
import io
import json
import shutil
import tarfile
import threading
import time
from collections import Counter
from email.utils import formatdate
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest
from conftest import grip, make_env_repo

ENTERPRISE = 'fixture-enterprise-credential-never-print'
PUBLIC = 'fixture-public-credential-never-print'


def archive_bytes():
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode='w:gz') as archive:
        entry = tarfile.TarInfo('payload')
        data = b'verified payload\n'
        entry.size = len(data)
        archive.addfile(entry, io.BytesIO(data))
    return output.getvalue()


class HttpFixture:
    def __init__(self):
        self.archive = archive_bytes()
        self.mode = 'healthy'
        self.requests = []
        self.counts = Counter()
        self.redirect_header = False
        self.public_leak = False
        owner = self
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                path = self.path.split('?', 1)[0]
                owner.requests.append(path)
                owner.counts[path] += 1
                header = self.headers.get('Authorization', '')
                owner.public_leak |= PUBLIC in header
                authorized = header == f'Bearer {ENTERPRISE}'
                if path == '/redirected':
                    owner.redirect_header |= bool(header)
                if path.startswith('/api/v3/repos/'):
                    if not authorized:
                        return self.respond(401, b'authentication required')
                    data = json.dumps({'tag_name': 'v1', 'assets': [{
                        'name': 'pkg-1.tar.gz', 'browser_download_url': owner.base + '/browser',
                        'url': owner.base + '/api-asset',
                    }]}).encode()
                    return self.respond(200, data, 'application/json')
                if path == '/browser':
                    return self.respond(200, b'<html>login</html>', 'text/html')
                if path == '/api-asset':
                    if not authorized:
                        return self.respond(401, b'authentication required')
                    if owner.mode == 'redirect':
                        return self.respond(302, b'', headers={'Location': owner.base.replace('127.0.0.1', 'localhost') + '/redirected'})
                if owner.mode == 'always-500':
                    return self.respond(500, b'upstream failure')
                if owner.mode == 'flaky' and owner.counts[path] < 3:
                    return self.respond(500 if owner.counts[path] == 1 else 503, b'upstream failure')
                if owner.mode == 'cooldown':
                    return self.respond(429, b'try later', headers={'Retry-After': '120'})
                if owner.mode == 'date-cooldown':
                    return self.respond(503, b'try later', headers={'Retry-After': formatdate(time.time() + 120, usegmt=True)})
                if owner.mode == 'partial' and owner.counts[path] == 1:
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/octet-stream')
                    self.send_header('Content-Length', str(len(owner.archive)))
                    self.end_headers()
                    self.wfile.write(owner.archive[:len(owner.archive) // 2])
                    self.wfile.flush()
                    self.close_connection = True
                    return
                self.respond(200, owner.archive, 'application/octet-stream')

            def respond(self, status, body, media='text/plain', headers=None):
                self.send_response(status)
                self.send_header('Content-Type', media)
                self.send_header('Content-Length', str(len(body)))
                for name, value in (headers or {}).items():
                    self.send_header(name, value)
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self):
        return f'http://127.0.0.1:{self.server.server_port}'

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


@pytest.fixture
def http_fixture(monkeypatch):
    for name in ['GH_TOKEN', 'GITHUB_TOKEN', 'GH_HOST', 'GITHUB_HOST', 'GH_ENTERPRISE_TOKEN', 'GITHUB_ENTERPRISE_TOKEN']:
        monkeypatch.delenv(name, raising=False)
    fixture = HttpFixture()
    try:
        yield fixture
    finally:
        fixture.close()


def tarball_repo(sandbox, server, names=('tool',), digest=None):
    pin = f", '{digest}'" if digest else ''
    return make_env_repo(sandbox / 'env', {name: f'''import {{module,tarball,trackedCopy}} from '@gripsack/core';
export default module('{name}',{{fetch:tarball('{server.base}/{name}'{pin}),install:{{payload:trackedCopy('~/.{name}')}}}});'''
        for name in names})


def test_transient_responses_retry_but_do_not_publish_in_check(sandbox, http_fixture):
    server = http_fixture
    server.mode = 'flaky'
    repo = tarball_repo(sandbox, server)
    result = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert result.returncode == 1, result.stdout + result.stderr
    assert server.counts['/tool'] == 3
    assert not (repo / 'locks/testhost.lock').exists()
    assert not (sandbox / '.local/share/gripsack/store').exists()


def test_exhausted_retries_name_attempt_count_and_keep_secrets_out(sandbox, http_fixture):
    server = http_fixture
    server.mode = 'always-500'
    repo = tarball_repo(sandbox, server)
    module = repo / 'modules/tool.ts'
    module.write_text(module.read_text().replace('/tool\'', '/tool?access_token=PRIVATE-QUERY-CANARY\''))
    result = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert result.returncode == 2
    assert server.counts['/tool'] == 3
    assert 'policy attempts 3' in result.stdout and 'attempt limit' in result.stdout
    assert 'PRIVATE-QUERY-CANARY' not in result.stdout + result.stderr
    for log in (sandbox / '.local/share/gripsack/runs').glob('*.jsonl'):
        assert 'PRIVATE-QUERY-CANARY' not in log.read_text()


def test_known_cooldown_accounts_for_other_modules_without_hammering_host(sandbox, http_fixture):
    server = http_fixture
    server.mode = 'cooldown'
    repo = tarball_repo(sandbox, server, ('alpha', 'omega'))
    result = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert result.returncode == 2
    assert server.counts['/alpha'] == 1 and server.counts['/omega'] == 0
    assert 'omega failed' in result.stdout and 'policy attempts 0' in result.stdout


def test_http_date_cooldown_is_not_clamped_to_an_early_retry(sandbox, http_fixture):
    server = http_fixture
    server.mode = 'date-cooldown'
    repo = tarball_repo(sandbox, server)
    result = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert result.returncode == 2 and server.counts['/tool'] == 1
    assert 'retry-wait budget' in result.stdout


@pytest.mark.parametrize('limited', [False, True])
def test_partial_body_restarts_from_zero_and_cannot_reset_byte_budget(sandbox, http_fixture, limited):
    server = http_fixture
    server.mode = 'partial'
    repo = tarball_repo(sandbox, server)
    if limited:
        with (repo / 'env.toml').open('a') as output:
            output.write(f'\n[settings]\ndownload_limit_bytes = {len(server.archive)}\n')
    result = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert result.returncode == (2 if limited else 1), result.stdout + result.stderr
    assert server.counts['/tool'] == 2
    if limited:
        assert 'byte cap' in result.stdout
    assert not (repo / 'locks/testhost.lock').exists()
    assert not (sandbox / '.local/share/gripsack/store').exists()


def test_hash_mismatch_is_not_retried(sandbox, http_fixture):
    repo = tarball_repo(sandbox, http_fixture, digest='0' * 64)
    result = grip('update', '--check', '--host', 'testhost', cwd=repo)
    assert result.returncode == 2
    assert http_fixture.counts['/tool'] == 1
    assert not (sandbox / '.local/share/gripsack/store').exists()


def test_explicit_host_binding_controls_resolution_and_locked_cold_api_fetch(sandbox, http_fixture, monkeypatch):
    server = http_fixture
    monkeypatch.setenv('GH_ENTERPRISE_TOKEN', ENTERPRISE)
    monkeypatch.setenv('GH_TOKEN', PUBLIC)
    repo = make_env_repo(sandbox / 'env', f'''import {{module,githubRelease,trackedCopy}} from '@gripsack/core';
export default module('private',{{fetch:githubRelease({{repo:'acme/tool',asset:'pkg-{{version.bare}}.tar.gz',base_url:'{server.base}'}}),install:{{payload:trackedCopy('~/.private')}}}});''')
    for host in [None, 'different.invalid']:
        if host is not None:
            monkeypatch.setenv('GH_HOST', host)
        result = grip('update', '--check', '--host', 'testhost', cwd=repo)
        assert result.returncode == 2
        assert 'GH_HOST=127.0.0.1' in result.stdout
        assert 'policy attempts 1' in result.stdout
        assert ENTERPRISE not in result.stdout + result.stderr
    monkeypatch.setenv('GH_HOST', '127.0.0.1')
    result = grip('update', '--host', 'testhost', cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    lock = (repo / 'locks/testhost.lock').read_bytes()
    assert '/api-asset' in server.requests and '/browser' not in server.requests
    # Cache is unrooted: no apply/generation exists yet.
    assert not (sandbox / '.local/share/gripsack/current').exists()
    shutil.rmtree(sandbox / '.local/share/gripsack/store')
    server.requests.clear()
    monkeypatch.delenv('GH_HOST')
    result = grip('apply', '--host', 'testhost', cwd=repo)
    assert result.returncode != 0
    assert server.requests == ['/browser']
    assert 'HTML/login' in result.stdout + result.stderr
    assert 'GH_HOST=127.0.0.1' in result.stdout + result.stderr
    monkeypatch.setenv('GH_HOST', '127.0.0.1')
    server.mode = 'redirect'
    server.requests.clear()
    result = grip('apply', '--host', 'testhost', cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    assert server.requests == ['/api-asset', '/redirected']
    assert not server.redirect_header and not server.public_leak
    assert (sandbox / '.private').read_bytes() == b'verified payload\n'
    assert (repo / 'locks/testhost.lock').read_bytes() == lock
    for log in (sandbox / '.local/share/gripsack/runs').glob('*.jsonl'):
        assert ENTERPRISE not in log.read_text() and PUBLIC not in log.read_text()
