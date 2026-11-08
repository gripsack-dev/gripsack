/-!
# Axiom audit (plan/0056 §7)

Every theorem under `Gripsack.Kernel` is audited: its transitive axioms must
be a subset of Lean's standard three. Anything else — `sorryAx`, a project
axiom, an Aeneas `axiom` standing in for an unmodelled external — fails the
build. The headline claims are also required to exist, so a deleted or
renamed theorem cannot pass silently.
-/
import Kernel
open Lean Elab Command

/-- Lean's standard axioms. -/
def allowedAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

/-- The headline claims: K3 (CLASSIFY-001), K4 retention admission, the
journal publication transition and mutation authority. -/
def claims : List Name := [
  ``Gripsack.Kernel.classify_committed_iff,
  ``Gripsack.Kernel.classify_uncommitted_iff,
  ``Gripsack.Kernel.classify_ambiguous_iff,
  ``Gripsack.Kernel.TransactionId.same_spec,
  ``Gripsack.Kernel.strictly_ascending_spec,
  ``Gripsack.Kernel.admit_gc_spec,
  ``Gripsack.Kernel.plan_prune_spec,
  ``Gripsack.Kernel.plan_delete_spec,
  ``Gripsack.Kernel.PublicationAction.successor_spec,
  ``Gripsack.Kernel.RecordPublication.acknowledge_spec,
  ``Gripsack.Kernel.RecordPublication.finish_spec,
  ``Gripsack.Kernel.admit_mutation_spec,
]

run_cmd do
  let env ← getEnv
  let theorems := env.constants.fold (init := #[]) fun acc name info =>
    if (`Gripsack.Kernel).isPrefixOf name && !name.isInternal && info matches .thmInfo _
    then acc.push name else acc
  let theorems := theorems.qsort (fun a b => a.toString < b.toString)
  if theorems.isEmpty then throwError "axiom audit: no theorems under Gripsack.Kernel"
  for name in theorems do
    let (_, state) := ((CollectAxioms.collect name).run env).run {}
    let axioms := state.axioms.qsort (fun a b => a.toString < b.toString)
    logInfo m!"AXIOMS {name}: {axioms.toList}"
    for axiom in axioms do
      unless allowedAxioms.contains axiom do
        throwError "axiom audit: {name} depends on {axiom}, which is not a standard Lean axiom"
  for claim in claims do
    unless theorems.contains claim do
      throwError "axiom audit: required claim {claim} is missing or is not a theorem"
  logInfo m!"AXIOM_AUDIT ok: {theorems.size} theorems, {claims.length} required claims, axioms ⊆ {allowedAxioms}"
