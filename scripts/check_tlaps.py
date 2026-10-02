#!/usr/bin/env python3
"""Fresh transaction/recovery induction with attributed protocol and gate negatives."""
from pathlib import Path
import copy
import hashlib
import json
import os
import shutil
import subprocess
import tempfile

from tlaps_catalog import load_catalog
from tlaps_evidence import Evidence, EvidenceError
from tlaps_source import SourceError, theorem_ranges

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
        TLAPM, '--strict', '--prefer-stdlib', '--nofp', '--stretch', '4',
        '--toolbox', str(start), str(end), '--threads', '2',
        '--cache-dir', str(cache), source.name,
    ], source.parent)
    return result.returncode, Evidence.parse(result.stdout)


def must_reject(label: str, check) -> None:
    try:
        check()
    except (EvidenceError, SourceError):
        print('calibration: ' + label + ' refused', flush=True)
        return
    raise EvidenceError('gate accepted ' + label)


def generalized_proofs(work: Path) -> tuple[Path, int]:
    manifest = ROOT / 'specs/recovery-proofs.json'
    catalog = load_catalog(manifest, ROOT / 'specs')
    directory = work / 'generalized'
    directory.mkdir()
    for source in catalog.sources:
        shutil.copy2(source, directory / source.name)
    print('TLAPS_GENERALIZED_CATALOG_SHA256=' + hashlib.sha256(manifest.read_bytes()).hexdigest(), flush=True)
    print('TLAPS_GENERALIZED_SOURCE_SHA256=' + json.dumps({
        source.name: hashlib.sha256((directory / source.name).read_bytes()).hexdigest()
        for source in catalog.sources
    }, sort_keys=True), flush=True)
    total = 0
    for unit in catalog.units:
        source = directory / (unit.module + '.tla')
        status, evidence = prove(source, work / ('generalized-cache-' + unit.module))
        evidence.positive(status, unit.minimum_obligations, source, unit.theorems)
        total += evidence.count
        print('TLAPS_GENERALIZED_UNIT=' + json.dumps({
            'module': unit.module, 'obligations': evidence.count, 'theorems': unit.theorems,
        }), flush=True)
    print(f'TLAPS_GENERALIZED_MODULES={len(catalog.units)}', flush=True)
    print(f'TLAPS_GENERALIZED_OBLIGATIONS={total}', flush=True)
    return directory, total


def catalog_calibrations(work: Path, proved: Path) -> None:
    original = json.loads((ROOT / 'specs/recovery-proofs.json').read_text())
    manifest = work / 'negative-catalog.json'
    mutations = []
    missing_import = copy.deepcopy(original)
    del missing_import['units']['FunctionFrames']
    mutations.append(('unchecked imported theorem module', missing_import))
    missing_name = copy.deepcopy(original)
    missing_name['units']['UndoCellProofs']['theorems'][0] = 'UnregisteredGhostTheorem'
    mutations.append(('wrong named theorem inventory', missing_name))
    empty_floor = copy.deepcopy(original)
    empty_floor['units']['UndoCellProofs']['minimum_obligations'] = 0
    mutations.append(('zero proof obligation floor', empty_floor))
    missing_root = copy.deepcopy(original)
    missing_root['roots'].remove('GeneralRecoveryProofs')
    mutations.append(('unreachable required proof unit', missing_root))
    for label, document in mutations:
        manifest.write_text(json.dumps(document))
        must_reject(label, lambda: load_catalog(manifest, proved))

    # Comments and quoted text are source input to the inventory lexer, not
    # theorem declarations. Hiding a required module's proofs must not satisfy
    # the frozen catalog through matching words in its comment.
    commented = work / 'commented-proofs'
    shutil.copytree(proved, commented)
    source = commented / 'FunctionFrames.tla'
    source.write_text('(*\n' + source.read_text() + '\n*)\n')
    manifest.write_text(json.dumps(original))
    must_reject('commented-out imported proofs', lambda: load_catalog(manifest, commented))
    print('TLAPS_INVENTORY_CALIBRATIONS=5', flush=True)


def restore_barrier_calibration(work: Path, proved: Path) -> None:
    directory = work / 'missing-restore-barrier'
    directory.mkdir()
    for name in ('UndoCell.tla', 'SelectionLifecycle.tla', 'JournalLifecycle.tla'):
        shutil.copy2(proved / name, directory / name)
    for name in ('RepeatedRecovery.tla', 'RecoveryBarrierWitness.tla'):
        shutil.copy2(ROOT / 'specs' / name, directory / name)
    source = directory / 'UndoCell.tla'
    original = source.read_text()
    before = '[c EXCEPT !.durable.live = c.cached.live, !.control.processed = TRUE]'
    if original.count(before) != 1:
        raise EvidenceError('restore-barrier mutation does not uniquely match')
    source.write_text(original.replace(before, '[c EXCEPT !.control.processed = TRUE]'))
    witness = directory / 'RecoveryBarrierWitness.tla'
    status, evidence = prove(witness, work / 'restore-barrier-cache')
    evidence.positive(status, 7, witness, (
        'RestoreBarrierWitnessStartsInSafeState', 'MissingRestoreBarrierDestroysRecoveryEvidence',
    ))
    print(f'TLAPS_RESTORE_BARRIER_WITNESS={evidence.count}', flush=True)
    shutil.copy2(ROOT / 'specs/cfg/repeated-transaction-premature-cleanup.cfg', directory / 'counterexample.cfg')
    counterexample = run([
        'java', '-Xmx2g', '-XX:+UseParallelGC', '-cp', TLC_JAR, 'tlc2.TLC', '-cleanup', '-workers', '2',
        '-metadir', str(work / 'restore-counterexample'), '-config', 'counterexample.cfg', 'RepeatedRecovery.tla',
    ], directory, 180)
    if (counterexample.returncode != 12
            or 'Invariant RecoveryEvidencePreserved is violated' not in counterexample.stdout):
        raise EvidenceError('missing restore barrier did not destroy actual recovery evidence')
    print('calibration: missing restore barrier has a proved violating transition and a reachable recovery-evidence loss', flush=True)


def session_fence_calibrations(work: Path, proved: Path) -> None:
    directory = work / 'session-fence-witness'
    directory.mkdir()
    shutil.copy2(proved / 'BuildSession.tla', directory / 'BuildSession.tla')
    source = directory / 'BuildSessionFenceWitness.tla'
    shutil.copy2(ROOT / 'specs' / source.name, source)
    status, evidence = prove(source, work / 'session-fence-cache')
    evidence.positive(status, 11, source, (
        'WorkerWitnessStartsSafe', 'MissingWorkerFenceAcceptsForeignReply',
        'ConflictWitnessStartsSafe', 'MissingConflictFenceAdmitsChangedTerminal',
    ))
    for config, invariant in (
        ('build-session-stale-worker.cfg', 'ForeignRefusal'),
        ('build-session-conflicting-replay.cfg', 'ConflictRefusal'),
    ):
        shutil.copy2(ROOT / 'specs/cfg' / config, directory / config)
        result = run([
            'java', '-Xmx1g', '-XX:+UseParallelGC', '-cp', TLC_JAR, 'tlc2.TLC',
            '-cleanup', '-workers', '1', '-metadir', str(work / 'session-fence-states'),
            '-config', config, 'BuildSession.tla',
        ], directory, 120)
        if result.returncode != 12 or f'Invariant {invariant} is violated' not in result.stdout:
            raise EvidenceError('session fence calibration did not violate ' + invariant)
    print(f'TLAPS_SESSION_FENCE_WITNESS={evidence.count}', flush=True)
    print('calibration: worker identity and terminal-conflict fences have proved violating transitions and reachable counterexamples', flush=True)

def worker_lease_calibrations(work: Path, proved: Path) -> None:
    directory = work / 'worker-lease-witness'
    directory.mkdir()
    shutil.copy2(proved / 'WorkerLease.tla', directory / 'WorkerLease.tla')
    source = directory / 'WorkerLeaseFenceWitness.tla'
    shutil.copy2(ROOT / 'specs' / source.name, source)
    status, evidence = prove(source, work / 'worker-lease-cache')
    evidence.positive(status, 50, source, (
        'LiveReadyStartsSafe', 'MissingStopFenceStopsWithLiveLease',
        'MissingLeaseFenceErasesCrashEvidence',
        'StaleEpochStateStartsSafe', 'MissingEpochFenceRetiresNewEpochRoot',
        'ForeignOwnerStateStartsSafe', 'MissingOwnerFenceRetiresNewOwnerRoot',
    ))
    for index, (config, invariant) in enumerate((
        ('worker-lease-early-stop.cfg', 'NoStopWithLiveLease'),
        ('worker-lease-crash-wipes.cfg', 'NoSilentLeaseVanish'),
        ('worker-lease-retire-foreign-owner.cfg', 'RetireRespectsOwner'),
        ('worker-lease-retire-stale-epoch.cfg', 'RetireRespectsEpoch'),
    )):
        shutil.copy2(ROOT / 'specs' / 'cfg' / config, directory / config)
        result = run([
            'java', '-Xmx1g', '-XX:+UseParallelGC', '-cp', TLC_JAR, 'tlc2.TLC',
            '-cleanup', '-workers', '1', '-metadir', str(work / f'worker-lease-states-{index}'),
            '-config', config, 'WorkerLease.tla',
        ], directory, 180)
        if result.returncode != 12 or f'Invariant {invariant} is violated' not in result.stdout:
            raise EvidenceError('worker lease calibration did not violate ' + invariant)
    print(f'TLAPS_WORKER_LEASE_WITNESS={evidence.count}', flush=True)
    print('calibration: worker stop/crash/owner/epoch fences have proved violating transitions and reachable counterexamples', flush=True)


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
        proved, _ = generalized_proofs(work)
        catalog_calibrations(work, proved)
        restore_barrier_calibration(work, proved)
        session_fence_calibrations(work, proved)
        worker_lease_calibrations(work, proved)
        print('tlaps gate: OK', flush=True)


if __name__ == '__main__':
    try:
        main()
    except (EvidenceError, SourceError, subprocess.TimeoutExpired) as error:
        raise SystemExit('FAIL: ' + str(error))
