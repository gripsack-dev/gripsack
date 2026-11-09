import Lean
import Kernel

/-!
# Axiom audit (plan/0056 §7)

Every theorem under `Gripsack.Kernel` is audited: its transitive axioms must
be a subset of Lean's standard three. Anything else — `sorryAx`, a project
axiom, an Aeneas `axiom` standing in for an unmodelled external — fails the
build. The headline claims are also required to exist, so a deleted or
renamed theorem cannot pass silently.
-/
open Lean Elab Command

/-- Lean's standard axioms. -/
def allowedAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

/-- The headline claims: K3 (CLASSIFY-001), K4 retention admission, the
journal publication transition and mutation authority, and the recover
machine (F2 step 2): the restore guard, unrestorable refusal, cleanup order
and classification before restore. -/
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
  ``Gripsack.Kernel.decide_spec,
  ``Gripsack.Kernel.step_spec,
  ``Gripsack.Kernel.mutate_guard,
  ``Gripsack.Kernel.unrestorable_refused,
  ``Gripsack.Kernel.mutate_never_unsupported,
  ``Gripsack.Kernel.marker_unlink_after_record_barrier,
  ``Gripsack.Kernel.mutate_after_classification,
]

/-- Every theorem under `Gripsack.Kernel`, sorted by name. -/
def kernelTheorems (env : Environment) : Array Name :=
  let audited : Name := `Gripsack.Kernel
  (env.constants.fold (init := (#[] : Array Name)) fun acc decl info =>
    if audited.isPrefixOf decl && decl.isInternal = false && info.isThm then acc.push decl
    else acc).qsort (fun a b => a.toString < b.toString)

-- `name` is a reserved token once the Aeneas library is imported (its attribute
-- syntax), so declarations are called `decl` here.
def auditAxioms : CommandElabM Unit := do
  let env ← getEnv
  let sorted := kernelTheorems env
  if sorted.isEmpty then
    throwError "axiom audit: no theorems under Gripsack.Kernel"
  for decl in sorted do
    let axioms := (← collectAxioms decl).qsort (fun a b => a.toString < b.toString)
    logInfo m!"AXIOMS {decl}: {axioms.toList}"
    for ax in axioms do
      unless allowedAxioms.contains ax do
        throwError "axiom audit: {decl} depends on {ax}, which is not a standard Lean axiom"
  for claim in claims do
    unless sorted.contains claim do
      throwError "axiom audit: required claim {claim} is missing or is not a theorem"
  logInfo m!"AXIOM_AUDIT ok: {sorted.size} theorems, {claims.length} required claims, axioms ⊆ {allowedAxioms}"

run_cmd auditAxioms
