#!/usr/bin/env python3
"""Production Verus gate with counted families and attributable semantic mutants."""
from dataclasses import dataclass
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from verus_evidence import Evidence, EvidenceError, self_check

ROOT = Path(__file__).resolve().parent.parent
CRATE = ROOT / "crates/gripsack-policy"
MIN_OBLIGATIONS = 111
# Actual successful SMT function queries, excluding generated clone/spec-only
# helpers. One family cannot disappear behind growth in an unrelated module.
FAMILIES = {
    "recovery": tuple("selection::" + name for name in (
        "TransactionId::from_bytes", "TransactionId::as_bytes",
        "SelectionIdentity::legacy", "SelectionIdentity::transaction",
        "SelectionIdentity::generation", "SelectionIdentity::transaction_id", "classify",
    )),
    "activation": tuple("activation::" + name for name in (
        "AttemptNumber::admit", "AttemptNumber::first", "AttemptNumber::value",
        "AttemptNumber::checked_next", "IntentState::attempt",
        "next_attempt", "finish_attempt", "supersede",
    )),
    "ownership": ("ownership::plan_copy", "ownership::plan_link"),
    "retention": tuple("retention::" + name for name in (
        "admit_gc", "contains_generation", "contains_identity", "plan_prune", "plan_delete",
        "lemma_delete_monotone", "lemma_delete_monotone_prefix",
    )),
    "generation-inventory": tuple("generation::" + name for name in (
        "GenerationId::new", "GenerationId::value", "GenerationId::checked_next",
        "GenerationId::checked_previous", "GenerationInventory::new",
        "GenerationInventory::as_slice", "GenerationList::new", "GenerationList::inventory",
    )),
    "merge": tuple("merge::" + name for name in (
        "splice_bytes", "lemma_rest_is_gaps", "lemma_splice_canonical", "lemma_splice_identity",
        "lemma_gaps_end_with_tail", "lemma_splice_preserves_edges",
    )),
    "merge-scanner": tuple("merge::scanner::" + name for name in (
        "span_text", "lemma_ascii_successor", "next_line_end", "scan_admits_splice", "scan",
    )),
    "graph": ("graph::build_closure", "graph::build_only_members", "graph::lemma_reachable_visited"),
    "graph-roles": ("graph::roles::project_graph_roles", "graph::roles::GraphRole::is_dependency"),
    "graph-name-index": tuple("graph::name_index::" + name for name in (
        "bind_output_index", "BoundOutputIndex::position", "distinct_names_have_distinct_indices",
    )),
    "target": ("target::release_at_most", "target::supports_target"),
    "scheduler": tuple("schedule::" + name for name in (
        "PureScheduler::new", "PureScheduler::start_next", "PureScheduler::finish_ok",
        "PureScheduler::finish_fail", "PureScheduler::failed", "lemma_unfinished_count_zero",
        "lemma_unfinished_count_zero_reverse", "lemma_unfinished_count_decrement",
        "lemma_unfinished_count_none_finished", "lemma_fresh_entry", "lemma_cast_collapse",
    )),
}


@dataclass(frozen=True)
class Mutant:
    name: str
    file: str
    function: str
    before: str
    after: str


MUTANTS = (
    Mutant("classifier", "selection.rs", "selection::classify", "(Some(_), _) => Classification::Ambiguous,", "(Some(_), _) => Classification::Committed,"),
    Mutant("classifier-transaction-identity", "selection.rs", "selection::classify",
           "(Some(_), Some(current)) if current == facts.target =>",
           "(Some(_), Some(current)) if current.generation() == facts.target.generation() =>"),
    Mutant("activation-interrupted-attempt", "activation.rs", "activation::next_attempt",
           "match attempt.checked_next() {", "match Some(*attempt) {"),
    Mutant("activation-stale-outcome", "activation.rs", "activation::finish_attempt",
           "if *active == attempt =>", "if *active != attempt =>"),
    Mutant("activation-supersession", "activation.rs", "activation::supersede",
           "IntentState::Started { attempt } => IntentState::Superseded { last_attempt: Some(*attempt) },",
           "IntentState::Started { attempt } => IntentState::Started { attempt: *attempt },"),
    Mutant("ownership-drift", "ownership.rs", "ownership::plan_copy", "Some((written, false)) if live == written => CopyPlan::Update,", "Some((written, true)) if live == written => CopyPlan::Update,"),
    Mutant("gc-roots-in-deletion", "retention.rs", "retention::plan_delete", "if !contains_identity(referenced, c) {", "if contains_identity(referenced, c) {"),
    Mutant("gc-newest-prefix", "retention.rs", "retention::plan_prune",
           "let g = generations[i];", "let g = generations[generations.len() - 1 - i];"),
    Mutant("generation-duplicate", "generation.rs", "generation::GenerationInventory::new",
           "if ids[i - 1].number >= ids[i].number {", "if ids[i - 1].number > ids[i].number {"),
    Mutant("merge-splice", "merge.rs", "merge::splice_bytes", "    out.extend_from_slice(&text[cursor..]);", ""),
    Mutant("merge-scanner-utf8", "merge/scanner.rs", "merge::scanner::scan",
           "span: Span { start: active.start, end },",
           "span: Span { start: active.start + 1, end },"),
    Mutant("graph-closure", "graph.rs", "graph::build_closure", "                    result.push(target);", ""),
    Mutant("graph-validation", "graph/roles.rs", "graph::roles::project_graph_roles", "RoleDecision { build: false, required_validation: true },", "RoleDecision { build: false, required_validation: false },"),
    Mutant("graph-name-index", "graph/name_index.rs", "graph::name_index::bind_output_index", "    let same_name = names[position] == declared;", "    let same_name = true;"),
    Mutant("target-abi", "target.rs", "target::supports_target", "Some(BinaryAbi::Gnu), Some(BinaryAbi::Gnu)", "Some(BinaryAbi::Gnu), Some(BinaryAbi::Musl)"),
    Mutant("scheduler-latch", "schedule.rs", "schedule::PureScheduler::start_next", "        if self.failed || self.head >= self.ready.len() {", "        if self.head >= self.ready.len() {"),
)


def verify(crate: Path, target: Path) -> tuple[int, Evidence]:
    command = [
        "cargo", "verus", "verify", "--manifest-path", str(crate / "Cargo.toml"),
        "-p", "gripsack-policy", "--locked", "--message-format=json", "--",
        "--output-json", "--time-expanded",
    ]
    print("RUNNER_COMMAND=" + " ".join(command), flush=True)
    result = subprocess.run(
        command, cwd=crate, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, timeout=600, env={**os.environ, "CARGO_TARGET_DIR": str(target)},
    )
    try:
        evidence = Evidence.parse(result.stdout)
    except EvidenceError:
        print(result.stdout, flush=True)
        raise
    print("VERUS_RESULT=" + json.dumps(evidence.results, sort_keys=True), flush=True)
    for diagnostic in evidence.diagnostics:
        if diagnostic.get("level") == "error":
            print("VERUS_DIAGNOSTIC=" + json.dumps(diagnostic, sort_keys=True), flush=True)
    return result.returncode, evidence


def copy_crate(destination: Path) -> Path:
    # Preserve the complete workspace and its exact lock graph. Making a
    # standalone package changes Cargo's root lock entry and can silently
    # re-resolve transitive tools; these mutants change only policy source.
    destination.mkdir()
    for name in ("Cargo.toml", "Cargo.lock"):
        shutil.copy2(ROOT / name, destination / name)
    for name in ("crates", "fuzz"):
        shutil.copytree(ROOT / name, destination / name, symlinks=True,
                        ignore=shutil.ignore_patterns("target", "__pycache__"))
    return destination / "crates/gripsack-policy"


def main() -> None:
    self_check()
    with tempfile.TemporaryDirectory(prefix="gripsack-verus-") as directory:
        temporary = Path(directory)
        status, evidence = verify(CRATE, temporary / "positive-target")
        evidence.positive(status, MIN_OBLIGATIONS, FAMILIES)
        print(f"positive: verification results:: {evidence.results['verified']} verified, 0 errors", flush=True)
        for family, required in FAMILIES.items():
            print(f"VERUS_FAMILY={family} checked={len(required)} functions={','.join(required)}", flush=True)
        for mutation in MUTANTS:
            crate = copy_crate(temporary / mutation.name)
            path = crate / "src" / mutation.file
            source = path.read_text()
            if source.count(mutation.before) != 1:
                raise EvidenceError(f"{mutation.name}: source mutation does not uniquely match")
            path.write_text(source.replace(mutation.before, mutation.after))
            status, evidence = verify(crate, temporary / (mutation.name + "-target"))
            evidence.mutant(status, path, mutation.function, crate.parent.parent)
            print(f"calibration: {mutation.name} rejected in {mutation.function}", flush=True)
        # A real unrelated failing lemma in the same source file cannot
        # impersonate the classifier mutant's named production failure.
        crate = copy_crate(temporary / "unrelated-lemma")
        path = crate / "src/selection.rs"
        source = path.read_text()
        prefix, suffix = source.rsplit("}", 1)
        path.write_text(prefix + "pub proof fn unrelated_calibration_failure() ensures false {}\n}" + suffix)
        status, evidence = verify(crate, temporary / "unrelated-target")
        evidence.mutant(status, path, "selection::unrelated_calibration_failure", crate.parent.parent)
        try:
            evidence.mutant(status, path, "selection::classify", crate.parent.parent)
        except EvidenceError:
            print("calibration: unrelated lemma refused as classifier evidence", flush=True)
        else:
            raise EvidenceError("unrelated lemma falsely calibrated classifier")
        print(f"VERUS_CALIBRATIONS={len(MUTANTS) + 1}", flush=True)
        print("verify gate: OK", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (EvidenceError, subprocess.TimeoutExpired) as error:
        raise SystemExit(f"FAIL: {error}")
