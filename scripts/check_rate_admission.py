#!/usr/bin/env python3
"""Real throttle observers, persistence and bounded waits with attributable mutants.

These deterministic adapter cases complement the exact-credit Verus contracts.
No fuzzing, saved corpus replay, clock theorem or global cross-process rate claim.
"""
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
THROTTLE = Path('crates/gripsack-fetch/src/throttle.rs')
PERSISTENCE = Path('crates/gripsack-fetch/src/throttle/persistence.rs')
PREFIX = 'throttle::'
EXPECTED = (
    'tests::a_backward_wall_clock_does_not_refill_the_same_interval_twice',
    'tests::fractional_declarations_retain_their_exact_refill_boundary',
    'tests::rate_admission_rejects_nonfinite_and_unfillable_buckets',
    'tests::plugin_declaration_replaces_builtin_default',
    'tests::token_wait_cannot_outlive_or_reset_an_operation_deadline',
    'tests::user_override_beats_plugin_declaration',
    'tests::url_host_extraction',
    'tests::persisted_token_and_time_bounds_cannot_panic_or_mint_extra_tokens',
    'persistence::tests::a_saved_fractional_timestamp_cannot_refill_an_already_accounted_interval',
    'tests::capability_registration_consumes_the_same_contended_deadline',
    'persistence::tests::changed_period_preserves_a_saved_partial_token_instead_of_resetting_the_burst',
    'tests::mutex_contention_cannot_outlive_or_grant_after_the_original_deadline',
    'tests::bucket_enforces_the_budget',
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
    Mutant('invalid-observation-normalized', THROTTLE,
           'rate_limit::admit_binary_rate(n.to_bits(), period)',
           'rate_limit::admit_binary_rate(n.abs().max(1.0).to_bits(), period)',
           'tests::rate_admission_rejects_nonfinite_and_unfillable_buckets',
           'invalid_rate_admitted'),
    Mutant('declared-period-substitution', THROTTLE,
           '"m" | "min" | "minute" => rate_limit::RatePeriod::Minute,',
           '"m" | "min" | "minute" => rate_limit::RatePeriod::Second,',
           'tests::fractional_declarations_retain_their_exact_refill_boundary',
           'declared_period_was_changed'),
    Mutant('mutex-deadline-extension', THROTTLE,
           '.try_lock_until(end)', '.try_lock_until(end + Duration::from_secs(600))',
           'tests::mutex_contention_cannot_outlive_or_grant_after_the_original_deadline',
           'deadline_blocked_by_throttle_lock'),
    Mutant('backward-clock-refill-reuse', THROTTLE,
           '        if let Ok(elapsed) = now.duration_since(self.updated) {\n'
           '            self.tokens.refill(elapsed.as_nanos());\n'
           '            self.updated = now;\n'
           '        }',
           '        let elapsed = now.duration_since(self.updated).unwrap_or_default();\n'
           '        self.tokens.refill(elapsed.as_nanos());\n'
           '        self.updated = now;',
           'tests::a_backward_wall_clock_does_not_refill_the_same_interval_twice',
           'wall_clock_interval_reused'),
    Mutant('persisted-time-truncation', PERSISTENCE,
           'updated_nanoseconds: updated.subsec_nanos(),', 'updated_nanoseconds: 0,',
           'persistence::tests::a_saved_fractional_timestamp_cannot_refill_an_already_accounted_interval',
           'persisted_timestamp_refilled_consumed_interval'),
    Mutant('persisted-period-substitution', PERSISTENCE,
           'let period = period(saved.period_ns)?;', 'let period = period(1_000_000_000)?;',
           'persistence::tests::changed_period_preserves_a_saved_partial_token_instead_of_resetting_the_burst',
           'persisted_period_reset_burst'),
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
    with tempfile.TemporaryDirectory(prefix='gripsack-rate-admission-') as temporary:
        temporary = Path(temporary)
        tree = temporary / 'source'
        tree.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / name, tree / name)
        for name in ('crates', 'fuzz', 'typescript', 'schema'):
            shutil.copytree(ROOT / name, tree / name, ignore=shutil.ignore_patterns(
                'target', 'node_modules', '.venv', '__pycache__',
            ))
        inputs = [Path('Cargo.toml'), Path('Cargo.lock'), THROTTLE, PERSISTENCE,
                  Path('crates/gripsack-policy/src/rate_limit.rs')]
        inputs += [path.relative_to(tree) for path in
                   (tree / 'crates/gripsack-policy/src/rate_limit').rglob('*.rs')]
        print('RATE_ADMISSION_INPUT_SHA256=' + json.dumps({str(path): hashlib.sha256(
            (tree / path).read_bytes()).hexdigest() for path in sorted(inputs)}, sort_keys=True), flush=True)
        target = temporary / 'target'
        positive = run(tree, target, PREFIX)
        summary = re.search(r'test result: ok\. (\d+) passed; 0 failed; 0 ignored', positive.stdout)
        if positive.returncode or summary is None or int(summary[1]) < len(EXPECTED):
            raise SystemExit('FAIL: complete production rate-admission cases did not pass')
        for case in EXPECTED:
            if f'test {PREFIX}{case} ... ok' not in positive.stdout:
                raise SystemExit(f'FAIL: missing rate-admission case: {case}')
        print(f'RATE_ADMISSION_PROPERTIES={len(EXPECTED)}', flush=True)
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
                raise SystemExit(f'FAIL: {mutant.name} missed its named concrete oracle')
            print(f'calibration: {mutant.name} rejected by {case}', flush=True)
        print(f'RATE_ADMISSION_MUTANTS={len(MUTANTS)}', flush=True)
        print('rate admission gate: OK', flush=True)


if __name__ == '__main__':
    try:
        main()
    except subprocess.TimeoutExpired as error:
        raise SystemExit('FAIL: timed-out rate admission is not semantic calibration') from error
