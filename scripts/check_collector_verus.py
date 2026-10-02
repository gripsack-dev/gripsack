#!/usr/bin/env python3
"""Check the production typed v6 collector, separately from policy closures.

The output walk uses vstd's explicit String-key BTreeMap ordering/iteration
model precondition. No source-completeness premise, callback, axiom, or copied
input grammar is admitted. Proven equality is of complete reference metadata
sets (not duplicate multiplicity or diagnostic order); runtime ordering is
preserved by the production walk and exercised by its admission smoke.
"""
from pathlib import Path
import tempfile

from check_verus import ROOT, Mutant, copy_crate, verify
from verus_evidence import EvidenceError

CRATE = ROOT / "crates/gripsack-ir"
MIN_OBLIGATIONS = 30
FAMILIES = {
    "typed-output-collector": tuple("workspace_v6::graph::" + name for name in (
        "references", "push", "Collector::edge", "Collector::named",
        "Collector::argument", "Collector::arguments", "Collector::environment",
        "Collector::command", "Collector::steps", "Collector::file", "Collector::collect",
    )),
    "typed-source-inputs": tuple("workspace_v6::graph::input::" + name for name in (
        "argument_input_reference", "source_input_references", "output_source_input_references",
    )),
    "typed-catalog-binding": (
        "workspace_v6::graph::kinds::kind_names",
        "workspace_v6::graph::binding::output_name",
        "workspace_v6::graph::binding::bind_reference",
        "workspace_v6::graph::binding::output_kind",
    ),
}
MUTANTS = (
    Mutant("collector-missing-tree-source", "workspace_v6/graph.rs",
           "workspace_v6::graph::Collector::file",
           'Some(WorkspaceSource::Tree { output, .. }) => self.edge(edges, output,\n                kind_names(Kinds::Artifact), &file.span, GraphRole::Runtime, None),',
           'Some(WorkspaceSource::Tree { .. }) => {},'),
    Mutant("collector-substituted-pixi-lock-input", "workspace_v6/graph/input.rs",
           "workspace_v6::graph::input::source_input_references",
           'Some(InputReference { to: &value.lock, at, site: InputSite::PixiLock })',
           'Some(InputReference { to: &value.manifest, at, site: InputSite::PixiLock })'),
    Mutant("collector-missing-image-entrypoint", "workspace_v6/graph.rs",
           "workspace_v6::graph::Collector::collect",
           '                self.arguments(edges, &value.config.entrypoint, &value.span, GraphRole::Runtime, Some(&value.target));',
           ''),
    Mutant("collector-substituted-owner-index", "workspace_v6/graph/binding.rs",
           "workspace_v6::graph::binding::bind_reference",
           'bind_output_index(names, output_name(edge.from), from)?',
           'bind_output_index(names, edge.to, from)?'),
)


def check(temporary: Path) -> None:
    status, evidence = verify(CRATE, temporary / "collector-positive-target")
    evidence.positive(status, MIN_OBLIGATIONS, FAMILIES)
    print(f"collector positive: {evidence.results['verified']} verified, 0 errors", flush=True)
    print("COLLECTOR_MODEL_PRECONDITION=vstd::std_specs::btree::key_obeys_cmp_spec::<String>()", flush=True)
    for family, required in FAMILIES.items():
        print(f"VERUS_FAMILY={family} checked={len(required)} functions={','.join(required)}", flush=True)
    for mutation in MUTANTS:
        crate = copy_crate(temporary / mutation.name, "gripsack-ir")
        path = crate / "src" / mutation.file
        source = path.read_text()
        if source.count(mutation.before) != 1:
            raise EvidenceError(f"{mutation.name}: source mutation does not uniquely match")
        path.write_text(source.replace(mutation.before, mutation.after))
        status, evidence = verify(crate, temporary / (mutation.name + "-target"))
        expected = (mutation.function, *mutation.also_fails)
        evidence.mutant(status, path, expected, crate.parent.parent)
        print(f"calibration: {mutation.name} rejected in {','.join(expected)}", flush=True)
    print(f"VERUS_COLLECTOR_CALIBRATIONS={len(MUTANTS)}", flush=True)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="gripsack-collector-verus-") as directory:
        check(Path(directory))
