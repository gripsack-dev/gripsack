#!/usr/bin/env python3
"""Mutant calibration for the `proofs` gate (plan/0056 §7).

Every moved kernel function gets at least one semantic mutation of its Rust
source. For each: copy the crate, apply the mutation, translate it exactly as
the gate does (proofs/extract.sh), build the theorems against the mutated
translation in a scratch copy of the lake project, and require the build to
FAIL inside the named theorem(s) — no other theorem may fail, and the
translation itself must still type-check. A proof that survives its mutant
proves nothing; a failure anywhere else is noise, not calibration.
"""
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
PROOFS = ROOT / "proofs"
KERNEL = ROOT / "crates" / "gripsack-kernel"


@dataclass(frozen=True)
class Mutant:
    name: str
    file: str
    before: str
    after: str
    fails: tuple[str, ...]


MUTANTS = (
    # CLASSIFY-001: numeric/generation identity must never grant commit authority
    Mutant("classify-generation-commit", "selection.rs",
           "            if current.same(&facts.target) {\n                Classification::Committed",
           "            if current.generation().same(facts.target.generation()) {\n                Classification::Committed",
           ("Gripsack.Kernel.classify_committed_iff", "Gripsack.Kernel.classify_uncommitted_iff",
            "Gripsack.Kernel.classify_ambiguous_iff")),
    # a fresh machine's interrupted first run is uncommitted, not ambiguous
    Mutant("classify-fresh-ambiguous", "selection.rs",
           "            None => Classification::Uncommitted,",
           "            None => Classification::Ambiguous,",
           ("Gripsack.Kernel.classify_uncommitted_iff", "Gripsack.Kernel.classify_ambiguous_iff")),
    # current = previous ≠ target is uncommitted; swapping the arms commits a crashed roll-forward
    Mutant("classify-previous-commit", "selection.rs",
           "                        if current.same(&previous) {\n                            Classification::Uncommitted",
           "                        if current.same(&previous) {\n                            Classification::Committed",
           ("Gripsack.Kernel.classify_committed_iff", "Gripsack.Kernel.classify_uncommitted_iff")),
    # transaction identity is byte-exact: ignoring the last byte is a different identity
    Mutant("transaction-identity-prefix", "selection.rs",
           "        while i < TRANSACTION_ID_BYTES {\n            if self.bytes[i] != other.bytes[i] {",
           "        while i + 1 < TRANSACTION_ID_BYTES {\n            if self.bytes[i] != other.bytes[i] {",
           ("Gripsack.Kernel.TransactionId.same_loop_spec",)),
    # K4: nothing is planned while recovery/activation is pending
    Mutant("gc-recovery-pending-ignored", "retention.rs",
           "    if recovery_pending {\n        return GcAdmission::RecoveryPending;\n    }",
           "    if recovery_pending && current.is_none() {\n        return GcAdmission::RecoveryPending;\n    }",
           ("Gripsack.Kernel.admit_gc_spec",)),
    # K4: a current generation without a directory is corruption, never admission
    Mutant("gc-corrupt-current-admitted", "retention.rs",
           "            if contains_generation(inventory.as_slice(), current) {\n                GcAdmission::Admitted\n            } else {\n                GcAdmission::CorruptCurrent\n            }",
           "            if contains_generation(inventory.as_slice(), current) {\n                GcAdmission::Admitted\n            } else {\n                GcAdmission::Admitted\n            }",
           ("Gripsack.Kernel.admit_gc_spec",)),
    # K4: prune the OLDEST excess, never the newest retained generations
    Mutant("gc-newest-prefix", "retention.rs",
           "        let candidate = generations[i];",
           "        let candidate = generations[generations.len() - 1 - i];",
           ("Gripsack.Kernel.plan_prune_loop_spec",)),
    # K4: the current generation is never pruned
    Mutant("gc-prunes-current", "retention.rs",
           "        if !names_generation(current, candidate) {\n            pruned.push(candidate);\n        }",
           "        pruned.push(candidate);",
           # `current` leaves the loop's signature, so the top-level proof breaks too
           ("Gripsack.Kernel.plan_prune_loop_spec", "Gripsack.Kernel.plan_prune_spec")),
    # K4: deletions ⊆ candidates − roots; inverting deletes exactly the roots
    Mutant("gc-roots-in-deletion", "retention.rs",
           "        if !contains_bytes(referenced, &candidates[i]) {",
           "        if contains_bytes(referenced, &candidates[i]) {",
           ("Gripsack.Kernel.plan_delete_loop_spec",)),
    # inventory admission must refuse duplicates (strict, not weak, ascent)
    Mutant("generation-duplicate", "generation.rs",
           "        if ids[i - 1].number >= ids[i].number {",
           "        if ids[i - 1].number > ids[i].number {",
           ("Gripsack.Kernel.strictly_ascending_loop_spec",)),
    # journal: the file barrier cannot be skipped
    Mutant("journal-publication-file-barrier", "journal.rs",
           "            Self::Stage => Self::SyncFile,",
           "            Self::Stage => Self::PublishName,",
           ("Gripsack.Kernel.PublicationAction.successor_spec",)),
    # journal: a failed effect is never accepted
    Mutant("journal-publication-failed-ack", "journal.rs",
           "        if !succeeded || !action.same(self.next) {",
           "        if !action.same(self.next) {",
           ("Gripsack.Kernel.RecordPublication.acknowledge_spec",)),
    # journal: authority only after the namespace barrier completed
    Mutant("journal-publication-early-authority", "journal.rs",
           "        if self.next.same(PublicationAction::Complete) {",
           "        if self.next.same(PublicationAction::PublishName) {",
           ("Gripsack.Kernel.RecordPublication.finish_spec",)),
    # journal: marker OR entry is not marker AND entry
    Mutant("journal-mutation-role-confusion", "journal.rs",
           "    if marker.role.same(RecordRole::RunMarker) && entry.role.same(RecordRole::MutationEntry) {",
           "    if marker.role.same(RecordRole::RunMarker) || entry.role.same(RecordRole::MutationEntry) {",
           ("Gripsack.Kernel.admit_mutation_spec",)),
)

# lake reports `error: <path>.lean:<line>:<col>: <message>`
ERROR = re.compile(r"^error:\s*(?P<file>[^\s:]+\.lean):(?P<line>\d+):(?P<col>\d+):", re.M)
DECLARATION = re.compile(r"^(?:@\[[^\]]*\]\s*)?(?:private\s+|protected\s+)?(?:theorem|lemma|def|abbrev|instance)\s+(?P<name>[^\s:(]+)", re.M)
NAMESPACE = re.compile(r"^namespace\s+(?P<name>\S+)", re.M)


def declarations(path: Path) -> list[tuple[int, str]]:
    """(line, fully qualified name) of every declaration in a theorem file."""
    namespace = ""
    found = []
    for number, line in enumerate(path.read_text().splitlines(), start=1):
        if match := NAMESPACE.match(line):
            namespace = match.group("name") + "."
            continue
        if line.startswith("end "):
            namespace = ""
            continue
        if match := DECLARATION.match(line):
            found.append((number, namespace + match.group("name")))
    return found


def attribute(file: Path, line: int) -> str | None:
    enclosing = None
    for start, name in declarations(file):
        if start <= line:
            enclosing = name
    return enclosing


def failing_theorems(output: str, project: Path) -> tuple[set[str], list[str]]:
    failures: set[str] = set()
    unattributed: list[str] = []
    for match in ERROR.finditer(output):
        file = (project / match.group("file")).resolve()
        try:
            relative = file.relative_to(project)
        except ValueError:
            unattributed.append(match.group(0))
            continue
        if relative.parts[0] != "Kernel" or not file.exists():
            unattributed.append(str(relative))
            continue
        name = attribute(file, int(match.group("line")))
        if name is None:
            unattributed.append(str(relative))
        else:
            failures.add(name)
    return failures, unattributed


def run(command: list[str], cwd: Path) -> subprocess.CompletedProcess:
    return subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, timeout=1800)


class Calibration(Exception):
    """One mutant's verdict: the message is the gate line, `log` the tool output."""

    def __init__(self, message: str, log: str = ""):
        super().__init__(message)
        self.log = log


def calibrate(mutant: Mutant, scratch: Path) -> str:
    """Return the calibration line, or raise `Calibration` with the evidence."""
    crate = scratch / "crate"
    shutil.copytree(KERNEL, crate, ignore=shutil.ignore_patterns("target"))
    source = crate / "src" / mutant.file
    text = source.read_text()
    if text.count(mutant.before) != 1:
        raise Calibration(f"FAIL {mutant.name}: mutation site does not match exactly once")
    source.write_text(text.replace(mutant.before, mutant.after))

    extraction = run(["sh", str(PROOFS / "extract.sh"), str(crate), str(scratch / "Extracted")], ROOT)
    if extraction.returncode != 0:
        raise Calibration(f"FAIL {mutant.name}: the mutated kernel did not translate", extraction.stdout)

    project = scratch / "proofs"
    project.mkdir()
    for name in ("lakefile.toml", "lean-toolchain", "lake-manifest.json", "Kernel.lean", "Audit.lean"):
        shutil.copy2(PROOFS / name, project / name)
    shutil.copytree(PROOFS / "Kernel", project / "Kernel")
    shutil.copytree(scratch / "Extracted", project / "Extracted")
    (project / ".lake").mkdir()
    os.symlink(PROOFS / ".lake" / "packages", project / ".lake" / "packages")

    build = run(["lake", "build"], project)
    if build.returncode == 0:
        raise Calibration(f"FAIL {mutant.name}: the proofs still build against the mutated kernel")
    failures, unattributed = failing_theorems(build.stdout, project)
    if unattributed:
        raise Calibration(f"FAIL {mutant.name}: errors outside the theorem files: {unattributed}", build.stdout)
    expected = set(mutant.fails)
    if not failures or not expected <= failures:
        raise Calibration(f"FAIL {mutant.name}: expected {sorted(expected)} to fail, got {sorted(failures)}",
                          build.stdout + f"\nattributed failures: {sorted(failures)}")
    if failures - expected:
        raise Calibration(f"FAIL {mutant.name}: unexpected failing theorems {sorted(failures - expected)}",
                          build.stdout)
    return f"calibration: {mutant.name} rejected by {', '.join(sorted(failures))}"


def verdict(mutant: Mutant) -> tuple[bool, str]:
    """(calibrated, text to print) for one mutant, in its own scratch directory."""
    with tempfile.TemporaryDirectory(prefix=f"mutant-{mutant.name}-") as directory:
        try:
            return True, calibrate(mutant, Path(directory))
        except Calibration as error:
            return False, (error.log + "\n" if error.log else "") + str(error)


def jobs(count: int) -> int:
    """Concurrent calibrations (plan/0057 D5): each is one Charon/Aeneas run plus
    a scratch `lake build`; PROOF_MUTANT_JOBS overrides the default of 6."""
    configured = os.environ.get("PROOF_MUTANT_JOBS")
    return max(1, min(count, int(configured) if configured else 6))


def main() -> None:
    selected = set(sys.argv[1:])
    mutants = [m for m in MUTANTS if not selected or m.name in selected]
    if selected and len(mutants) != len(selected):
        raise SystemExit(f"unknown mutant(s): {sorted(selected - {m.name for m in mutants})}")
    # Every mutant runs even after a failure, so one gate run reports them all;
    # verdicts are printed in declaration order whatever order they finish in.
    failed: list[str] = []
    with ThreadPoolExecutor(max_workers=jobs(len(mutants))) as pool:
        for mutant, (calibrated, text) in zip(mutants, pool.map(verdict, mutants)):
            print(text, flush=True)
            if not calibrated:
                failed.append(mutant.name)
    if failed:
        raise SystemExit(f"FAIL: {len(failed)} mutant(s) not calibrated: {failed}")
    print(f"PROOF_CALIBRATIONS={len(mutants)}", flush=True)


if __name__ == "__main__":
    main()
