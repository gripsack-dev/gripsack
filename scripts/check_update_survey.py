#!/usr/bin/env python3
"""Complete update reports, real cache/lock effects and attributable mutations."""
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
UPDATE = Path('crates/gripsack-exec/src/update.rs')
REPORT = Path('crates/gripsack-exec/src/report.rs')
METADATA_CASE = 'update::tests::check_and_publish_agree_on_metadata_only_pin_changes'
COMPLETE_CASE = 'update::tests::selected_survey_preserves_every_result_and_only_complete_publish_changes_lock'
EXPECTED = (
    'report::update_model::every_survey_order_preserves_error_over_change_precedence',
    METADATA_CASE,
    COMPLETE_CASE,
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
    Mutant('missing-selected-observation', UPDATE,
           'for name in missing {', 'for name in missing.into_iter().take(0) {',
           COMPLETE_CASE, 'selected_survey_did_not_account_for_all_modules'),
    Mutant('failure-status-projection', REPORT,
           'UpdateStatus::Failed { .. } => UpdateDisposition::Failed,',
           'UpdateStatus::Failed { .. } => UpdateDisposition::Changed,',
           COMPLETE_CASE, 'survey_disposition_counts_misclassified'),
    Mutant('partial-entry-equality', UPDATE,
           'let unchanged = old_entry == Some(&prepared.entry);',
           'let unchanged = old.is_some_and(|old| old.sha256 == pin.sha256);',
           METADATA_CASE, 'check_full_entry_change_was_missed'),
    Mutant('check-source-publication', UPDATE,
           'if mode == UpdateMode::Publish {', 'if true {',
           COMPLETE_CASE, 'check_published_source_cache'),
    Mutant('check-lock-publication', UPDATE,
           'if survey.summary().publishes_lock(mode) {',
           'if survey.summary().publishes_lock(UpdateMode::Publish) {',
           METADATA_CASE, 'check_published_lock'),
    Mutant('dropped-lock-effect', UPDATE,
           '        crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;', '',
           METADATA_CASE, 'published lock missing'),
    Mutant('premature-lock-effect', UPDATE,
           '                lock.modules.insert(name.clone(), prepared.entry);',
           '                lock.modules.insert(name.clone(), prepared.entry);\n'
           '                crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;',
           COMPLETE_CASE, 'failed_update_partially_published_lock'),
)


def run(root: Path, target: Path, case: str):
    command = ['cargo', 'test', '--locked', '-p', 'gripsack-exec', '--lib',
               case, '--', '--exact', '--nocapture']
    print('RUNNER_COMMAND=' + ' '.join(command), flush=True)
    result = subprocess.run(command, cwd=root,
                            env={**os.environ, 'CARGO_TARGET_DIR': str(target)},
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, timeout=600)
    print(result.stdout, flush=True)
    return result


def main():
    with tempfile.TemporaryDirectory(prefix='gripsack-update-survey-') as temporary:
        temporary = Path(temporary)
        tree = temporary / 'source'
        tree.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / name, tree / name)
        for name in ('crates', 'fuzz', 'typescript', 'schema'):
            shutil.copytree(ROOT / name, tree / name, ignore=shutil.ignore_patterns(
                'target', 'node_modules', '.venv', '__pycache__',
            ))
        inputs = [Path('Cargo.toml'), Path('Cargo.lock'), UPDATE, REPORT,
                  Path('crates/gripsack-exec/src/update/prepare.rs'),
                  Path('crates/gripsack-exec/src/update/tests.rs'),
                  Path('crates/gripsack-exec/src/source/preflight.rs'),
                  Path('crates/gripsack-exec/src/workspace/mod.rs'),
                  Path('crates/gripsack-policy/src/update_survey.rs')]
        print('UPDATE_SURVEY_INPUT_SHA256=' + json.dumps({str(path): hashlib.sha256(
            (tree / path).read_bytes()).hexdigest() for path in sorted(inputs)}, sort_keys=True), flush=True)
        target = temporary / 'target'
        for case in EXPECTED:
            result = run(tree, target, case)
            if (result.returncode or f'test {case} ... ok' not in result.stdout
                    or re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored', result.stdout) is None):
                raise SystemExit(f'FAIL: update survey property did not pass: {case}')
        print(f'UPDATE_SURVEY_PROPERTIES={len(EXPECTED)}', flush=True)
        for mutant in MUTANTS:
            source = tree / mutant.source
            original = source.read_text()
            if original.count(mutant.before) != 1:
                raise SystemExit(f'FAIL: {mutant.name} no longer uniquely matches')
            source.write_text(original.replace(mutant.before, mutant.after))
            try:
                negative = run(tree, target, mutant.case)
            finally:
                source.write_text(original)
            if (negative.returncode == 0
                    or f'test {mutant.case} ... FAILED' not in negative.stdout
                    or mutant.diagnostic not in negative.stdout
                    or 'test result: FAILED. 0 passed; 1 failed; 0 ignored' not in negative.stdout
                    or 'could not compile' in negative.stdout):
                raise SystemExit(f'FAIL: {mutant.name} missed its named update effect oracle')
            print(f'calibration: {mutant.name} rejected by {mutant.case}', flush=True)
        print(f'UPDATE_SURVEY_MUTANTS={len(MUTANTS)}', flush=True)
        print('Update survey gate: OK', flush=True)


if __name__ == '__main__':
    try:
        main()
    except subprocess.TimeoutExpired as error:
        raise SystemExit('FAIL: timed-out update verification is not semantic calibration') from error
