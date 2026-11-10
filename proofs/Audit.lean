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
journal publication transition, and the recover
machine (F2 step 2): the restore guard, unrestorable refusal, cleanup order
and classification before restore; and the per-unit ownership decision (K1):
writes only to absence, gripsack's own last write, or a consented object;
drift never written; identities matter only through equality; and the
rollback machine (F2 step 8): every write installs the target generation's
recorded identity (K2) into absence or gripsack's own write (K1), never inside
a generation; a restore is reported only when it landed; the flip follows
every unit, under a fresh transaction, onto a target that keeps its profile;
an unrestorable unit aborts; managed blocks (K5, F2 step 6): bytes outside
the owner's blocks are preserved exactly by upsert and removal, an upsert's
output upserts to itself and reads back as the rendered block, a block is
removed only if intact, a non-file host and conflicting mode evidence are
never written, and rollback restores a block only from a payload that is its
record; and the GC machine (F2 step 4, K4/K4′): no effect
while recovery or activation is pending or in a preview; a retired generation
is pruned, never current, and needed by no retained manifest; a removed
candidate is pinned by no retained manifest and named by no external root; the
retained generations are closed under predecessors; removal follows the
durable retirement; the journal run (F2 step 3): a mutation only right after its
destination's observation, durable prior capture and durable entry, in a run
whose marker is durable (P1); journal records removed only after the flip was
acknowledged or in a run that journaled nothing, and the marker only after a
barrier following every record unlink (P3); a unit accepted only when its
intended object landed; the activation machine (F2 step 5, P4 at the kernel
level): a hook runs only right after its durable `Started` record, while its
batch's exact selection is current and authorized, at most once per run, with
the attempt that replays its previous record under the same intent; terminal
records are never rewritten and never run; a returned hook is recorded as
what it returned and nothing is rolled back; a superseded batch runs nothing;
the receipt follows every intent and precedes the pointer's removal; and
generation numbering (K6): an allocated number exceeds the floor, the floor
never lowers, a publication is the allocated number above the floor, not yet
published, under a fresh transaction, and vacuous applies publish nothing. -/
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
  ``Gripsack.Kernel.decide_spec,
  ``Gripsack.Kernel.step_spec,
  ``Gripsack.Kernel.mutate_guard,
  ``Gripsack.Kernel.unrestorable_refused,
  ``Gripsack.Kernel.mutate_never_unsupported,
  ``Gripsack.Kernel.marker_unlink_after_record_barrier,
  ``Gripsack.Kernel.mutate_after_classification,
  ``Gripsack.Kernel.decide_unit_k1,
  ``Gripsack.Kernel.decide_unit_drift_never_written,
  ``Gripsack.Kernel.decide_unit_owned_drift_refused,
  ``Gripsack.Kernel.decide_unit_unrecorded_refused,
  ``Gripsack.Kernel.decide_unit_identity_only,
  ``Gripsack.Kernel.decide_unit_takeover_acquires,
  ``Gripsack.Kernel.decide_unit_progress,
  ``Gripsack.Kernel.Rollback.decide_restore_spec,
  ``Gripsack.Kernel.Rollback.decide_restore_drift_kept,
  ``Gripsack.Kernel.Rollback.decide_restore_unrestorable_refused,
  ``Gripsack.Kernel.Rollback.rollback_installs_target,
  ``Gripsack.Kernel.Rollback.rollback_write_k1,
  ``Gripsack.Kernel.Rollback.rollback_never_writes_generation,
  ``Gripsack.Kernel.Rollback.rollback_restored_landed,
  ``Gripsack.Kernel.Rollback.rollback_flip_after_every_unit,
  ``Gripsack.Kernel.Rollback.rollback_flip_admitted,
  ``Gripsack.Kernel.Rollback.rollback_unrestorable_aborts,
  ``Gripsack.Kernel.Rollback.rollback_delegated_refusal_aborts,
  ``Gripsack.Kernel.closure_spec,
  ``Gripsack.Kernel.rooted_spec,
  ``Gripsack.Kernel.Gc.gc_effects_admitted,
  ``Gripsack.Kernel.Gc.gc_preview_effect_free,
  ``Gripsack.Kernel.Gc.gc_retire_pruned,
  ``Gripsack.Kernel.Gc.gc_never_retires_current,
  ``Gripsack.Kernel.Gc.gc_retire_newest_first,
  ``Gripsack.Kernel.Gc.gc_remove_unreferenced,
  ``Gripsack.Kernel.Gc.gc_doomed_exact,
  ``Gripsack.Kernel.Gc.gc_retained_closed,
  ``Gripsack.Kernel.Gc.gc_remove_after_retirement,
  ``Gripsack.Kernel.Block.splice_ok,
  ``Gripsack.Kernel.Block.upsert_preserves_outside,
  ``Gripsack.Kernel.Block.remove_preserves_outside,
  ``Gripsack.Kernel.Block.upsert_idempotent,
  ``Gripsack.Kernel.Block.upsert_round_trip,
  ``Gripsack.Kernel.Block.decide_block_remove_intact,
  ``Gripsack.Kernel.Block.decide_block_foreign_untouched,
  ``Gripsack.Kernel.Block.decide_block_mode_evidence_preserves,
  ``Gripsack.Kernel.Block.decide_block_restore_exact,
  ``Gripsack.Kernel.JournalRun.step_spec,
  ``Gripsack.Kernel.JournalRun.mutate_after_durable_intent,
  ``Gripsack.Kernel.JournalRun.record_unlink_needs_no_evidence,
  ``Gripsack.Kernel.JournalRun.marker_unlink_after_record_barrier,
  ``Gripsack.Kernel.JournalRun.unit_accepted_only_landed,
  ``Gripsack.Kernel.fresh_spec,
  ``Gripsack.Kernel.Activation.next_attempt_spec,
  ``Gripsack.Kernel.Activation.finish_attempt_spec,
  ``Gripsack.Kernel.Activation.supersede_spec,
  ``Gripsack.Kernel.Activation.hook_after_durable_start,
  ``Gripsack.Kernel.Activation.hook_selection_current,
  ``Gripsack.Kernel.Activation.hook_once_per_run,
  ``Gripsack.Kernel.Activation.start_replays,
  ``Gripsack.Kernel.Activation.settle_returned,
  ``Gripsack.Kernel.Activation.terminal_never_rewritten,
  ``Gripsack.Kernel.Activation.terminal_never_runs,
  ``Gripsack.Kernel.Activation.superseded_runs_nothing,
  ``Gripsack.Kernel.Activation.supersede_writes_superseded,
  ``Gripsack.Kernel.Activation.leaves_terminal,
  ``Gripsack.Kernel.Activation.archive_after_every_intent,
  ``Gripsack.Kernel.Activation.clear_after_archive,
  ``Gripsack.Kernel.Publication.allocate_exceeds_floor,
  ``Gripsack.Kernel.Publication.floor_never_lowers,
  ``Gripsack.Kernel.Publication.publish_admitted,
  ``Gripsack.Kernel.Publication.vacuous_iff,
  ``Gripsack.Kernel.Apply.decide_apply_spec,
  ``Gripsack.Kernel.Apply.decide_apply_drift_never_written,
  ``Gripsack.Kernel.Apply.decide_apply_refused,
  ``Gripsack.Kernel.Apply.apply_installs_intended,
  ``Gripsack.Kernel.Apply.apply_write_k1,
  ``Gripsack.Kernel.Apply.apply_applied_landed,
  ``Gripsack.Kernel.Apply.apply_release_undeclared,
  ``Gripsack.Kernel.Apply.apply_no_deploy_after_release,
  ``Gripsack.Kernel.Apply.apply_flip_after_every_unit,
  ``Gripsack.Kernel.Apply.apply_flip_admitted,
  ``Gripsack.Kernel.Apply.apply_vacuous_untouched,
  ``Gripsack.Kernel.Apply.apply_refused_aborts,
  ``Gripsack.Kernel.Apply.apply_aborted_terminal,
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
