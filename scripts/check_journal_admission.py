#!/usr/bin/env python3
"""M-V4 deterministic production byte/recovery properties and semantic calibration.

No random generator, fuzz target or saved fuzz corpus is executed. A mutant
must reach the real recovery-effect assertion; build errors and timeouts fail.
"""
from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
MARKER = Path('crates/gripsack-store/src/journal/marker.rs')
PREFIX = 'journal::admission_tests::'
EFFECT = PREFIX + 'rejected_metadata_cannot_reach_destination_effects'
EXPECTED = (
    'required_marker_fields_never_default_to_fresh_state',
    'marker_scalar_boundaries_roundtrip_without_identity_loss',
    'marker_wrong_types_duplicates_and_overflow_do_not_admit_facts',
    'entry_versions_and_distinct_prior_states_roundtrip',
    'entry_missing_duplicate_version_and_scalar_bytes_are_rejected',
    'truncation_and_invalid_framing_never_admit_partial_records',
    'rejected_metadata_cannot_reach_destination_effects',
)
# Count actual executed table rows/prefixes, not merely seven test functions.
# Growing a family is allowed; shrinking one requires a deliberate review.
MINIMUMS = {
    'JOURNAL_REQUIRED_MARKER_FIELDS': 3,
    'JOURNAL_MARKER_ROUNDTRIPS': 24,
    'JOURNAL_MARKER_REJECTIONS': 29,
    'JOURNAL_ENTRY_ROUNDTRIPS': 20,
    'JOURNAL_ENTRY_REJECTIONS': 110,
    'JOURNAL_TRUNCATED_PREFIXES': 173,
    'JOURNAL_EFFECT_REJECTIONS': 139,
}


def run(root: Path, target: Path, selection: str, exact: bool = False):
    command = ['cargo', 'test', '--locked', '-p', 'gripsack-store', '--lib',
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
    with tempfile.TemporaryDirectory(prefix='gripsack-journal-admission-') as temporary:
        temporary = Path(temporary)
        tree = temporary / 'source'
        tree.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / name, tree / name)
        for name in ('crates', 'fuzz', 'typescript', 'schema'):
            shutil.copytree(ROOT / name, tree / name, ignore=shutil.ignore_patterns(
                'target', 'node_modules', '.venv', '__pycache__',
            ))
        target = temporary / 'target'
        positive = run(tree, target, PREFIX)
        if positive.returncode or 'test result: ok. 7 passed; 0 failed; 0 ignored' not in positive.stdout:
            raise SystemExit('FAIL: complete production journal admission property set did not pass')
        for case in EXPECTED:
            if f'test {PREFIX}{case} ... ok' not in positive.stdout:
                raise SystemExit(f'FAIL: named admission property did not execute: {case}')
        counts = dict((name, int(count)) for name, count in re.findall(
            r'\b(JOURNAL_[A-Z_]+)=(\d+)\b', positive.stdout))
        for name, minimum in MINIMUMS.items():
            if counts.get(name, 0) < minimum:
                raise SystemExit(f'FAIL: missing or narrowed byte-case family {name}')
        print('JOURNAL_ADMISSION_PROPERTIES=7', flush=True)

        source = tree / MARKER
        original = source.read_text()
        required = '.ok_or_else(|| A::Error::missing_field("previous_generation"))?'
        if original.count(required) != 1:
            raise SystemExit('FAIL: missing-previous mutation no longer uniquely matches')
        source.write_text(original.replace(required, '.unwrap_or(None)'))
        try:
            negative = run(tree, target, EFFECT, exact=True)
        finally:
            source.write_text(original)
        if (negative.returncode == 0
                or f'test {EFFECT} ... FAILED' not in negative.stdout
                or 'recovery_admitted_invalid_marker: missing_previous_generation' not in negative.stdout
                or 'test result: FAILED. 0 passed; 1 failed; 0 ignored' not in negative.stdout
                or 'could not compile' in negative.stdout):
            raise SystemExit('FAIL: missing previous did not fail the named production recovery-effect oracle')
        print('calibration: missing-previous-defaulted-to-null rejected by real recovery effects', flush=True)
        print('JOURNAL_ADMISSION_MUTANTS=1', flush=True)
        print('journal admission gate: OK', flush=True)


if __name__ == '__main__':
    try:
        main()
    except subprocess.TimeoutExpired as error:
        raise SystemExit('FAIL: timed-out admission verification is not semantic calibration') from error
