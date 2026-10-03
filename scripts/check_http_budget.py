#!/usr/bin/env python3
"""Deterministic HTTP budget/clock/effect oracles; no fuzz or corpus replay."""
from dataclasses import dataclass
from pathlib import Path
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
REQUEST = Path('crates/gripsack-fetch/src/http/request.rs')
RETRY = Path('crates/gripsack-fetch/src/http/retry.rs')
PREFIX = 'http::'
EXPECTED = (
    'body::tests::a_new_attempt_cannot_reset_consumed_transfer_bytes',
    'failure::tests::certificate_failures_are_not_transient_connections',
    'failure::tests::failure_locations_redact_credentials_and_queries',
    'retry::tests::an_expired_clock_observation_cannot_admit_an_attempt',
    'retry::tests::caller_deadline_bounds_attempt_completion_and_retry_wait',
    'retry::tests::real_retry_policy_has_fixed_deadline_and_bounded_attempts',
    'retry::tests::terminal_classes_and_upstream_cooldowns_never_get_fast_replays',
    'tests::host_binding_survives_url_normalization',
    'tests::bound_credentials_require_tls_and_registry_artifact_refuses_cleartext',
    'tests::credential_routing_model_uses_actual_host_selector',
    'tests::proxy_bypass_preserves_domain_and_port_boundaries',
    'request::tests::a_successful_body_consumer_cannot_complete_after_the_operation_deadline',
    'request::tests::cooldown_contention_cannot_extend_the_original_operation_deadline',
)


@dataclass(frozen=True)
class Mutant:
    name: str
    source: Path
    before: str
    after: str
    case: str
    diagnostic: str


MUTANTS = (
    Mutant('late-body-success', REQUEST,
           '                                budget\n'
           '                                    .complete(Instant::now())\n'
           '                                    .map_err(|stop| admission_failure(&budget, stop))?;',
           '',
           'request::tests::a_successful_body_consumer_cannot_complete_after_the_operation_deadline',
           'late_http_consumer_was_reported_successfully'),
    Mutant('ignored-clock-observation', RETRY,
           '    pub fn begin(&mut self, mut now: Instant) -> Result<Duration, RetryStopReason> {\n'
           '        loop {',
           '    pub fn begin(&mut self, mut now: Instant) -> Result<Duration, RetryStopReason> {\n'
           '        now = self.started;\n'
           '        loop {',
           'retry::tests::an_expired_clock_observation_cannot_admit_an_attempt',
           'expired_http_attempt_admitted'),
    Mutant('terminal-classification-replayed', RETRY,
           'kind.retryable(),', 'kind.retryable() || matches!(kind, HttpFailureKind::Tls),',
           'retry::tests::terminal_classes_and_upstream_cooldowns_never_get_fast_replays',
           'nonretryable_http_failure_replayed'),
    Mutant('cooldown-deadline-extension', REQUEST,
           '        if let Some(host) = &host {\n'
           '            let mut cooldowns = self\n'
           '                .cooldowns\n'
           '                .try_lock_until(budget.deadline())',
           '        if let Some(host) = &host {\n'
           '            let mut cooldowns = self\n'
           '                .cooldowns\n'
           '                .try_lock_until(budget.deadline() + Duration::from_secs(600))',
           'request::tests::cooldown_contention_cannot_extend_the_original_operation_deadline',
           'deadline_blocked_by_http_cooldown_lock'),
)


def run(root: Path, target: Path, selection: str, exact: bool = False):
    command = ['cargo', 'test', '--locked', '-p', 'gripsack-fetch', '--lib',
               selection, '--', '--nocapture']
    if exact:
        command.append('--exact')
    print('RUNNER_COMMAND=' + ' '.join(command), flush=True)
    result = subprocess.run(command, cwd=root,
                            env={**os.environ, 'CARGO_TARGET_DIR': str(target)},
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, timeout=600)
    print(result.stdout, flush=True)
    return result


def main():
    with tempfile.TemporaryDirectory(prefix='gripsack-http-budget-') as temporary:
        temporary = Path(temporary)
        tree = temporary / 'source'
        tree.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / name, tree / name)
        for name in ('crates', 'fuzz', 'typescript', 'schema'):
            shutil.copytree(ROOT / name, tree / name, ignore=shutil.ignore_patterns(
                'target', 'node_modules', '.venv', '__pycache__',
            ))
        inputs = [Path('Cargo.toml'), Path('Cargo.lock'), REQUEST, RETRY,
                  Path('crates/gripsack-fetch/src/http.rs'),
                  Path('crates/gripsack-fetch/src/http/body.rs'),
                  Path('crates/gripsack-fetch/src/http/failure.rs'),
                  Path('crates/gripsack-policy/src/operation_budget.rs'),
                  Path('crates/gripsack-policy/src/retry_budget.rs')]
        print('HTTP_BUDGET_INPUT_SHA256=' + json.dumps({str(path): hashlib.sha256(
            (tree / path).read_bytes()).hexdigest() for path in sorted(inputs)}, sort_keys=True), flush=True)
        target = temporary / 'target'
        positive = run(tree, target, PREFIX)
        summary = re.search(r'test result: ok\. (\d+) passed; 0 failed; 0 ignored', positive.stdout)
        if positive.returncode or summary is None or int(summary[1]) < len(EXPECTED):
            raise SystemExit('FAIL: complete HTTP budget adapter set did not pass')
        for case in EXPECTED:
            if f'test {PREFIX}{case} ... ok' not in positive.stdout:
                raise SystemExit(f'FAIL: missing HTTP budget adapter: {case}')
        print(f'HTTP_BUDGET_PROPERTIES={len(EXPECTED)}', flush=True)
        for mutant in MUTANTS:
            source = tree / mutant.source
            original = source.read_text()
            if original.count(mutant.before) != 1:
                raise SystemExit(f'FAIL: {mutant.name} no longer uniquely matches')
            source.write_text(original.replace(mutant.before, mutant.after))
            case = PREFIX + mutant.case
            try:
                negative = run(tree, target, case, exact=True)
            finally:
                source.write_text(original)
            if (negative.returncode == 0
                    or f'test {case} ... FAILED' not in negative.stdout
                    or mutant.diagnostic not in negative.stdout
                    or 'test result: FAILED. 0 passed; 1 failed; 0 ignored' not in negative.stdout
                    or 'could not compile' in negative.stdout):
                raise SystemExit(f'FAIL: {mutant.name} missed its named HTTP effect oracle')
            print(f'calibration: {mutant.name} rejected by {case}', flush=True)
        print(f'HTTP_BUDGET_MUTANTS={len(MUTANTS)}', flush=True)
        print('HTTP budget gate: OK', flush=True)


if __name__ == '__main__':
    try:
        main()
    except subprocess.TimeoutExpired as error:
        raise SystemExit('FAIL: timed-out HTTP budget verification is not semantic calibration') from error
