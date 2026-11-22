import Protocol.RecoveryReplay

/-! One restart at a cleanup cut. This consequence is conditional on the
*same admitted row* (including its original prior source), path interning,
mode-aware digest codec and a truthful re-observation under S4–S7. It applies
after uncommitted classification, including permitted no-marker admission;
it does not reclassify a committed row or assert repeated-crash convergence. -/
namespace Gripsack.Protocol.RecoveryRetry

open Gripsack.Storage Gripsack.Protocol.RecoverLeaf
open gripsack_kernel Aeneas Aeneas.Std Result

private theorem prior_object {f : recover.EntryFacts} {i : recover.Ident} (h : f.prior = .Obj i) :
    recover.decide f (.Obj i) = ok .Unchanged ∨
    (recover.decide f (.Obj i) = ok .Restore ∧ f.intended = .Obj i) := by
  obtain ⟨intends, hi, spec⟩ := Kernel.spec_exists_ok (Kernel.intends_spec f.intended i)
  obtain ⟨b, hm, eq⟩ := Kernel.spec_exists_ok (Kernel.Prior.matches_spec f.prior (.Obj i))
  have yes := eq.mpr (Or.inr ⟨i, h, rfl⟩)
  cases intends with
  | false => exact Or.inl (by simp [recover.decide, hi, hm, yes])
  | true => exact Or.inr ⟨by simp [recover.decide, hi], spec.mp rfl⟩

/-- `decide` tests the intended object *before* the original prior, so a
surviving row need not yield Unchanged even though the exact prior is live. -/
theorem decide_at_prior (k : Codec) (f : recover.EntryFacts) (prior : Value)
    (wire : f.prior = prior.prior k) :
    recover.decide f (prior.live k) = ok .Unchanged ∨
    (recover.decide f (prior.live k) = ok .Restore ∧
      ∃ i, prior.live k = .Obj i ∧ f.intended = .Obj i) := by
  cases prior with
  | absent => exact Or.inl (by simp [Value.live, Value.prior, recover.decide, wire])
  | file _ | link _ =>
    rcases prior_object wire with unchanged | ⟨restore, intended⟩
    · exact Or.inl unchanged
    · exact Or.inr ⟨restore, _, rfl, intended⟩

/-- Exactly the non-destructive choices available to this re-admitted row.
A Mutate can physically replace the inode, but not change the prior's file
contents/mode or symlink target. A foreign edit instead issues Preserve. -/
inductive Authorized (k : Codec) (idx : U32) (f : recover.EntryFacts) (prior observed current : Value) :
    recover.Command → Prop where
  | unchanged : current = prior →
      Authorized k idx f prior observed current (.Seal idx f.dest (current.live k) f.prior)
  | sameRestore : current = prior → (∃ i, current.live k = .Obj i ∧ f.intended = .Obj i) →
      Authorized k idx f prior observed current (.Mutate idx f.dest (current.live k) (.Restore f.prior))
  | foreign : current = observed → recover.decide f (current.live k) = ok .Preserve →
      Authorized k idx f prior observed current (.Preserve idx f.dest)

/-- This is about the actual host reply, not only the command's prior digest. -/
theorem Authorized.reproduces {k idx f prior observed current cmd ack l m t}
    (safe : Authorized k idx f prior observed current cmd)
    (reply : Reply k idx f l prior current cmd ack m t) : Durable t l current := by
  cases safe with
  | unchanged eq | sameRestore eq _ => exact eq ▸ reply.durable
  | foreign => exact reply.durable

private theorem dest_refl (d : ids.DestId) : ids.DestId.same d d = ok true := by
  obtain ⟨b, h, spec⟩ := Kernel.spec_exists_ok (Kernel.DestId.same_spec d d)
  simpa [spec.mpr rfl] using h

/-- With the same surviving facts the foreign branch issues exactly Preserve,
not a Mutate followed by some ostensibly benign effect. -/
theorem preserve_step {s : recover.RecoverState} {f : recover.EntryFacts} {live}
    (phase : s.phase = .Observing) (entry : s.entry = some f) (keep : recover.decide f live = ok .Preserve) :
    recover.recover_step s (.Observed f.dest live) =
      ok ({s with phase := .Preserving}, .Preserve s.index f.dest) := by
  simp [recover.recover_step, phase, recover.on_observed, entry, dest_refl, keep]

private theorem authorized_step {k f prior observed current s}
    (settled : current = prior ∨ (current = observed ∧ recover.decide f (observed.live k) = ok .Preserve))
    (wire : f.prior = prior.prior k) (phase : s.phase = recover.Phase.Observing) (entry : s.entry = some f) :
    ∃ next cmd, recover.recover_step s (.Observed f.dest (current.live k)) = ok (next,cmd) ∧
      Authorized k s.index f prior observed current cmd := by
  rcases settled with rfl | ⟨rfl, kept⟩
  · rcases decide_at_prior k f current wire with unchanged | ⟨restore, intended⟩
    · exact ⟨{s with phase := .Sealing}, _, by
        simp [recover.recover_step, phase, recover.on_observed, entry, dest_refl, unchanged], .unchanged rfl⟩
    · exact ⟨{s with phase := .Restoring}, _, by
        simp [recover.recover_step, phase, recover.on_observed, entry, dest_refl, restore], .sameRestore rfl intended⟩
  · exact ⟨_, _, preserve_step phase entry kept, .foreign rfl kept⟩

/-- The restarted reader's returned bytes are not an oracle for the last
outcome. `Holds.unique` identifies a successful concrete observation with
the persisted bytes/mode/binding in this kill or power image. `entry` uses
the unchanged facts of the re-admitted row; a different row is not covered. -/
theorem settled_row_restart {m rest l v prior observed current k f s kind}
    (stable : Durable m l v)
    (settled : v = prior ∨ (v = observed ∧ recover.decide f (observed.live k) = ok .Preserve))
    (wire : f.prior = prior.prior k) (cut : Crash m kind rest)
    (read : Holds rest.kind rest.fs.visible l current)
    (phase : s.phase = recover.Phase.Observing) (entry : s.entry = some f) :
    current = v ∧ ∃ next cmd, recover.recover_step s (.Observed f.dest (current.live k)) = ok (next,cmd) ∧
      Authorized k s.index f prior observed current cmd := by
  have same := read.unique ((stable.after_crash cut).holds _ (crash_visible _))
  exact ⟨same, authorized_step (same ▸ settled) wire phase entry⟩

/-- Rescanning after record unlinks may renumber entries and therefore their
host-local `DestId`. Undo semantics come from the surviving row's payload,
not its ordinal. The separately admitted leaf route still names the same site. -/
structure SameUndo (survivor original : recover.EntryFacts) : Prop where
  prior : survivor.prior = original.prior
  before : survivor.before = original.before
  intended : survivor.intended = original.intended

theorem SameUndo.decide {f g : recover.EntryFacts} (same : SameUndo f g) (live : recover.Live) :
    recover.decide f live = recover.decide g live := by
  cases live <;> simp [recover.decide, same.prior, same.before, same.intended]

/-- Image-level corollary consumes precisely the uncommitted leaf alternatives
in the cleanup theorem. `admitted` includes the actual record read and checked
original blob. A previous ordinal need not equal this invocation's `i`; only
the admitted location, undo values and their interpretation must agree. -/
theorem readmitted_row_step {k : RecoveryReplay.Context} {m : Machine} {i : U32}
    {old f : recover.EntryFacts} {s : recover.RecoverState} {observed current : Value}
    (settled : Holds m.kind m.fs.visible (k.row i).leaf (k.row i).prior ∨
      (recover.decide old (observed.live k.codec) = ok .Preserve ∧ Holds m.kind m.fs.visible (k.row i).leaf observed))
    (admitted : RecoveryReplay.Read k m (.ReadEntry i, .EntryRead f)) (same : SameUndo (k.row i).facts old)
    (wire : old.prior = (k.row i).prior.prior k.codec) (read : Holds m.kind m.fs.visible (k.row i).leaf current)
    (phase : s.phase = .Observing) (entry : s.entry = some f) (index : s.index = i) :
    ∃ next cmd, recover.recover_step s (.Observed f.dest (current.live k.codec)) = ok (next,cmd) ∧
      Authorized k.codec i f (k.row i).prior observed current cmd := by
  have facts : f = (k.row i).facts := admitted.1
  subst f
  subst i
  apply authorized_step _ (same.prior.trans wire) phase entry
  rcases settled with previous | ⟨kept, foreign⟩
  · exact Or.inl (read.unique previous)
  · exact Or.inr ⟨read.unique foreign, (same.decide _).trans kept⟩

/-- A separately retired row does not furnish a successful `ReadEntry`.
If a record survives but is malformed or fails to decode, this premise also
cannot be met; neither case synthesizes a new prior for this leaf. -/
theorem missing_record_not_admitted {k : RecoveryReplay.Context} {i : U32} {m : Machine}
    (missing : m.fs.visible.link (k.row i).recordLoc = none) (f : recover.EntryFacts) :
    ¬ RecoveryReplay.Read k m (.ReadEntry i, .EntryRead f) := by
  intro admitted
  have decoded := admitted.2
  simp [Image.read, missing] at decoded

end Gripsack.Protocol.RecoveryRetry
