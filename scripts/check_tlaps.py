#!/usr/bin/env python3
"""Full transaction induction plus attributed barrier-order and gate negatives."""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

from tlaps_evidence import Evidence, EvidenceError, theorem_ranges

ROOT = Path(__file__).resolve().parent.parent
TLAPM = os.environ.get('TLAPM', 'tlapm')
TLC_JAR = os.environ.get('TLC_JAR', '/tla/tla2tools.jar')
VERSION = '7824dab'
MIN_OBLIGATIONS = 301
THEOREMS = (
    'EntryType', 'RecordImage', 'CompletedStepDurability', 'RecoveryOfEntry', 'UndoIntended',
    'RecoveryProjection', 'InitImpliesInv', 'InductiveStep',
    'InvImpliesSafety', 'TransactionSafety',
)


def run(command: list[str], cwd: Path, timeout: int = 600) -> subprocess.CompletedProcess:
    print('RUNNER_COMMAND=' + ' '.join(command), flush=True)
    result = subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, timeout=timeout)
    print(result.stdout, flush=True)
    return result


def prepare(directory: Path) -> Path:
    directory.mkdir()
    for name in ('Transaction.tla', 'TransactionProofs.tla'):
        shutil.copy2(ROOT / 'specs' / name, directory / name)
    return directory / 'TransactionProofs.tla'


def prove(source: Path, cache: Path, theorem: str | None = None) -> tuple[int, Evidence]:
    start, end = (1, len(source.read_text().splitlines()))
    if theorem is not None:
        start, end = theorem_ranges(source)[theorem]
    result = run([
        TLAPM, '--strict', '--prefer-stdlib', '--nofp',
        '--toolbox', str(start), str(end), '--threads', '2',
        '--cache-dir', str(cache), source.name,
    ], source.parent)
    return result.returncode, Evidence.parse(result.stdout)


def must_reject(label: str, check) -> None:
    try:
        check()
    except EvidenceError:
        print('calibration: ' + label + ' refused', flush=True)
        return
    raise EvidenceError('gate accepted ' + label)


def main() -> None:
    version = subprocess.check_output([TLAPM, '--version'], text=True).strip()
    if version != VERSION:
        raise EvidenceError(f'wrong TLAPM revision: {version}')
    print('TLAPS_REVISION=' + version, flush=True)
    with tempfile.TemporaryDirectory(prefix='gripsack-tlaps-') as temporary:
        work = Path(temporary)
        source = prepare(work / 'positive')
        status, evidence = prove(source, work / 'positive-cache')
        evidence.positive(status, MIN_OBLIGATIONS, source, THEOREMS)
        print(f'TLAPS_PILOT_OBLIGATIONS={evidence.count}', flush=True)
        for name in THEOREMS:
            print('TLAPS_THEOREM=' + name, flush=True)

        mutant = prepare(work / 'missing-entry-barrier')
        transaction = mutant.parent / 'Transaction.tla'
        original = transaction.read_text()
        before = "    /\\ durable' = volatile'\n"
        after = "    /\\ durable' = IF step = 4 THEN durable ELSE volatile'\n"
        if original.count(before) != 1:
            raise EvidenceError('cleanup-barrier mutation does not uniquely match')
        transaction.write_text(original.replace(before, after))
        # A failed proof search can time out even for a false formula. Never
        # count that as calibration: prove an explicit violating transition
        # instead, without importing the now-invalid induction theorem.
        witness = mutant.parent / 'TransactionBarrierWitness.tla'
        shutil.copy2(ROOT / 'specs' / witness.name, witness)
        status, negative = prove(witness, work / 'barrier-cache')
        negative.positive(status, 1, witness, ('CleanupBarrierWitness',))
        print(f'TLAPS_BARRIER_WITNESS={negative.count}', flush=True)
        # TLC additionally establishes reachability and the observable failure.
        shutil.copy2(ROOT / 'specs/cfg/apply-deploy.cfg', mutant.parent / 'counterexample.cfg')
        counterexample = run([
            'java', '-Xmx1g', '-cp', TLC_JAR, 'tlc2.TLC', '-cleanup', '-workers', '1',
            '-metadir', str(work / 'tlc-states'), '-config', 'counterexample.cfg', 'Transaction.tla',
        ], mutant.parent, 120)
        if (counterexample.returncode == 0
                or 'Invariant Oracle is violated' not in counterexample.stdout):
            raise EvidenceError('missing barrier did not violate the actual Oracle')
        print('calibration: missing entry-cleanup barrier has a proved violating step and an Oracle counterexample', flush=True)

        unrelated = prepare(work / 'unrelated')
        text = unrelated.read_text()
        end = text.rfind('\n====')
        if end < 0:
            raise EvidenceError('proof module terminator missing')
        unrelated.write_text(text[:end] + '\nTHEOREM UnrelatedCalibration == FALSE\n  BY SMT\n' + text[end:])
        status, other = prove(unrelated, work / 'unrelated-cache', 'UnrelatedCalibration')
        other.mutant(status, unrelated, 'UnrelatedCalibration')
        must_reject('unrelated theorem as barrier evidence',
                    lambda: other.mutant(status, unrelated, 'CompletedStepDurability'))

        empty = prepare(work / 'empty')
        empty.write_text('---- MODULE TransactionProofs ----\nEXTENDS Transaction, TLAPS\n====\n')
        status, zero = prove(empty, work / 'empty-cache')
        if status == 0 or zero.count != 0:
            raise EvidenceError('strict prover did not reject an empty target')
        must_reject('empty proof as positive evidence',
                    lambda: zero.positive(status, MIN_OBLIGATIONS, empty, THEOREMS))
        print('TLAPS_PILOT_CALIBRATIONS=3', flush=True)
        print('tlaps gate: OK', flush=True)


if __name__ == '__main__':
    try:
        main()
    except (EvidenceError, subprocess.TimeoutExpired) as error:
        raise SystemExit('FAIL: ' + str(error))
