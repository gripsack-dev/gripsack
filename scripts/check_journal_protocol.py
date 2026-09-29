#!/usr/bin/env python3
"""Concrete journal permissions and filesystem-order correspondence; no fuzz/replay.

The filesystem oracle consumes actual primitive traces under the stated fsync
contract. Neither traces nor injected process failures certify physical disks.
"""
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
JOURNAL = Path('crates/gripsack-store/src/journal')
FILESYSTEM = Path('crates/gripsack-fs/src/publication.rs')
PREFIX = 'journal::protocol_tests::'
CASES = (
    'changed_destination_cannot_gain_a_mutation_permit',
    'permitted_mutation_uses_the_captured_parent',
    'mutation_error_retains_evidence_and_recovers_the_original',
    'unfinished_mutation_cannot_end_the_run',
)
PUBLICATION = 'persistence_model::atomic_file_link_and_tree_publication_are_ordered_at_every_cut'


@dataclass(frozen=True)
class Mutant:
    name: str
    source: Path
    before: str
    after: str
    crate: str
    case: str
    diagnostic: str


MUTANTS = (
    Mutant('capture-observation', JOURNAL / 'mutation.rs',
           'if observed != expected {', 'if false && observed != expected {',
           'gripsack-store', PREFIX + CASES[0], 'changed_destination_gained_mutation_authority'),
    Mutant('captured-parent-effect', JOURNAL / 'mutation.rs',
           'effect(&self.captured.directory, &self.captured.name)?;',
           'effect(&gripsack_fs::open(self.captured.destination.parent().unwrap())?, &self.captured.name)?;',
           'gripsack-store', PREFIX + CASES[1], 'captured_parent_lost_mutation_authority'),
    Mutant('mutation-error-precedence', JOURNAL / 'mutation.rs',
           'effect(&self.captured.directory, &self.captured.name)?;',
           'let _ = effect(&self.captured.directory, &self.captured.name);',
           'gripsack-store', PREFIX + CASES[2], 'mutation_error_was_lost'),
    Mutant('unfinished-run-cleanup', JOURNAL / 'marker.rs',
           'if name != RUN_MARKER\n            && Path::new(&name)',
           'if false && name != RUN_MARKER\n            && Path::new(&name)',
           'gripsack-store', PREFIX + CASES[3], 'unfinished_mutation_lost_its_run_marker'),
    Mutant('publication-file-barrier', FILESYSTEM,
           'operation(Boundary::FileSync, self.0.name, || self.0.file.sync_all())?;', '',
           'gripsack-fs', PUBLICATION, 'publish before file durability'),
    Mutant('publication-namespace-barrier', FILESYSTEM,
           'fsync_dir(self.directory, parent_rel(self.name))?;', '',
           'gripsack-fs', PUBLICATION, 'primitive returned before durability'),
)


def run(root: Path, target: Path, crate: str, selection: str, *, exact: bool):
    command = ['cargo', 'test', '--locked', '-p', crate, '--lib', selection, '--', '--show-output']
    if exact:
        command.append('--exact')
    print('RUNNER_COMMAND=' + ' '.join(command), flush=True)
    result = subprocess.run(command, cwd=root,
                            env={**os.environ, 'CARGO_TARGET_DIR': str(target)},
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, timeout=1200)
    print(result.stdout, flush=True)
    return result


def main():
    sources = [JOURNAL / name for name in ('mutation.rs', 'marker.rs', 'storage.rs', 'recover.rs', 'protocol_tests.rs')]
    sources.extend((FILESYSTEM, Path('crates/gripsack-fs/src/persistence_model.rs'),
                    Path('crates/gripsack-policy/src/journal_protocol.rs')))
    print('JOURNAL_PROTOCOL_SOURCE_SHA256=' + json.dumps({
        str(path): hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in sources
    }, sort_keys=True), flush=True)
    with tempfile.TemporaryDirectory(prefix='gripsack-journal-protocol-') as directory:
        temporary = Path(directory)
        tree = temporary / 'source'
        tree.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / name, tree / name)
        for name in ('crates', 'fuzz', 'typescript', 'schema'):
            shutil.copytree(ROOT / name, tree / name, symlinks=True,
                            ignore=shutil.ignore_patterns('target', 'node_modules', '.venv', '__pycache__'))
        target = temporary / 'target'
        positive = run(tree, target, 'gripsack-store', PREFIX, exact=False)
        if (positive.returncode or 'test result: ok. 4 passed; 0 failed; 0 ignored' not in positive.stdout
                or any(f'test {PREFIX}{case} ... ok' not in positive.stdout for case in CASES)):
            raise SystemExit('FAIL: complete real journal authority properties did not pass')
        publication = run(tree, target, 'gripsack-fs', PUBLICATION, exact=True)
        if (publication.returncode or f'test {PUBLICATION} ... ok' not in publication.stdout
                or 'test result: ok. 1 passed; 0 failed; 0 ignored' not in publication.stdout):
            raise SystemExit('FAIL: production filesystem-order oracle did not pass')
        print('JOURNAL_PROTOCOL_PROPERTIES=5', flush=True)
        for mutant in MUTANTS:
            path = tree / mutant.source
            original = path.read_text()
            if original.count(mutant.before) != 1:
                raise SystemExit(f'FAIL: {mutant.name}: source mutation does not uniquely match')
            path.write_text(original.replace(mutant.before, mutant.after))
            try:
                result = run(tree, target, mutant.crate, mutant.case, exact=True)
            finally:
                path.write_text(original)
            if (result.returncode == 0 or f'test {mutant.case} ... FAILED' not in result.stdout
                    or mutant.diagnostic not in result.stdout
                    or 'test result: FAILED. 0 passed; 1 failed; 0 ignored' not in result.stdout
                    or 'could not compile' in result.stdout):
                raise SystemExit(f'FAIL: {mutant.name}: missing attributable production failure')
            print(f'calibration: {mutant.name} rejected by {mutant.case}', flush=True)
        print(f'JOURNAL_PROTOCOL_MUTANTS={len(MUTANTS)}', flush=True)
        print('journal protocol gate: OK', flush=True)


if __name__ == '__main__':
    try:
        main()
    except subprocess.TimeoutExpired as error:
        raise SystemExit('FAIL: journal timeout is not semantic calibration') from error
