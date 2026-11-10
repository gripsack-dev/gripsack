import Extracted.GripsackKernel
import Kernel.Selection
import Kernel.Recover
import Kernel.Ownership
import Kernel.Directory
import Kernel.Retention

/-!
# The rollback machine (plan/0056 F2 steps 8 and 9)

Theorems over the Aeneas translation of `gripsack_kernel::rollback`
(`Extracted/GripsackKernel.lean`).

* `decide_restore_spec`: the per-unit decision writes only the target
  generation's recorded identity from retained bytes (or the removal where the
  target deploys nothing), only outside generation directories, and only into
  absence or over gripsack's own current write (the ownership decision without
  consent, K1); it never claims a restore.
* `decide_restore_drift_kept` (K1): a live object that is not gripsack's
  current write and is not already the target's identity is kept, never
  written.
* `decide_restore_unrestorable_refused`: a write the retained bytes cannot
  reproduce exactly refuses the rollback.
* `decide_restore_clear_refused`: a clear (a removal a directory shape change
  needs) over anything but gripsack's current write refuses the rollback.
* (K2) `rollback_installs_target`: over every run, a write is issued only for
  the unit under decision, from its observation, and installs exactly the
  target generation's recorded identity for that unit (or its removal).
* `rollback_write_k1`: over every run, a write replaces only absence or
  gripsack's own current write — never drift, never with consent assumed.
* `rollback_never_writes_generation`: over every run, no write targets a
  destination inside a published generation's directory; the flip (the only
  other effect) names the selection, not a generation's contents.
* `rollback_restored_landed`: over every run, a unit is reported restored
  only when the landed object is exactly the identity the write installed.
* `rollback_flip_after_every_unit`: over every run, the flip is issued only
  after every unit of the plan was reported settled (restored, already there,
  kept, untouched, delegated, held or a settled directory step) and every
  directory step commanded was reported settled, and names the planned
  selection.
* The directory fence (step 9): `rollback_write_unheld` — no write targets a
  destination the directory domain holds; `rollback_directory_unclaimed` — a
  directory step is commanded only for a destination no unit restores the
  target's record to, in stage order, and never for a hold;
  `rollback_directory_settled_answers` and `rollback_directory_refusal_aborts`
  — a step is reported settled only when the host settled it, and a step it
  cannot settle ends the rollback; `rollback_unit_out_of_stage_aborts` and
  `rollback_directory_out_of_stage_aborts`.
* `rollback_flip_admitted` (K6): over every run, the flip names a transaction
  selection whose transaction differs from the one it replaces, and the target
  retains its own shell profile (FND-2: nothing is written into it).
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.Rollback

/-! ## The per-unit decision -/

/-- What a write may install for a target record: its recorded identity, from
retained bytes, or — where the target deploys nothing — the release the
ownership epoch recorded (the removal, or the object gripsack took over).
Preserved drift records no write. -/
def Installs : rollback.Target → recover.Intended → Prop
  | .Released r, i => i = r
  | .Wrote o b, i => b = .Retained ∧ i = .Obj o
  | .Preserved, _ => False
  | .Cleared r, i => r = .Removed ∧ i = .Removed

/-- The landed object is what the write intended. -/
def Lands : recover.Intended → recover.Live → Prop
  | .Removed, l => l = .Absent
  | .Obj o, l => l = .Obj o

/-- A write the rollback may issue for unit `f` over the observed `live`. -/
def WriteOk (f : rollback.UnitFacts) (live : recover.Live) (i : recover.Intended) : Prop :=
  Installs f.target i ∧ f.place = .Outside ∧ (live = .Absent ∨ Ours f.current live)

/-- The unfolding the decision claims start from. -/
macro "restore_cases" : tactic => `(tactic| (
  simp only [rollback.decide_restore, rollback.authority, rollback.install, rollback.writing,
    rollback.withheld, rollback.retained, rollback.writable, rollback.clears, rollback.blocked,
    ownership.decide_unit, ownership.Recorded.wrote, ownership.acquires,
    ownership.Consent.grants_take_over, ownership.refusal, recover.intends, Ident.same_ok,
    decide_eq_true_eq]
  all_goals (try simp only [bind_tc_ok, bind_ok, decide_eq_true_eq])
  all_goals (try split_ifs)
  all_goals (repeat' split)
  all_goals simp_all [WriteOk, Installs, Ours, WP.spec_ok]))

/-- The decision writes only what `WriteOk` allows, and never claims a
restore: only a landed write is reported restored. -/
theorem decide_restore_spec (k : U32) (f : rollback.UnitFacts) (live : recover.Live) :
    rollback.decide_restore k f live ⦃ r =>
      (∀ i, r = .Write i → WriteOk f live i) ∧ r ≠ .Leave .Restored ⦄ := by
  obtain ⟨dest, place, route, current, target, when⟩ := f
  cases target with
  | Released r => cases r <;> cases place <;> cases live <;> cases current <;> restore_cases
  | Preserved => cases place <;> cases live <;> cases current <;> restore_cases
  | Wrote o b => cases b <;> cases place <;> cases live <;> cases current <;> restore_cases
  | Cleared r => cases r <;> cases place <;> cases live <;> cases current <;> restore_cases

/-- K1 through rollback: a live object that is not gripsack's current write
and is not already what the target records is kept, never written (a release
is authorized as a removal, so only gripsack's own write is ever released). -/
theorem decide_restore_drift_kept (k : U32) (f : rollback.UnitFacts) (o : recover.Ident)
    (hr : f.current ≠ .Wrote o) (ht : ∀ b, f.target ≠ .Wrote o b)
    (hc : ∀ r, f.target ≠ .Cleared r) :
    rollback.decide_restore k f (.Obj o) ⦃ r =>
      (∃ why, r = .Leave (.Kept why)) ∨ r = .Leave .Untouched ⦄ := by
  obtain ⟨dest, place, route, current, target, when⟩ := f
  simp only at hr ht hc
  cases target with
  | Released r => cases r <;> cases place <;> cases current <;> restore_cases
  | Preserved => cases place <;> cases current <;> restore_cases
  | Wrote t b =>
    have hto : t ≠ o := fun h => ht b (by rw [h])
    cases b <;> cases place <;> cases current <;> restore_cases
  | Cleared r => exact absurd rfl (hc r)

/-- A clear empties only gripsack's current write: over any other object it
refuses the rollback, naming the unit (the shape change cannot proceed). -/
theorem decide_restore_clear_refused (k : U32) (f : rollback.UnitFacts) (o : recover.Ident)
    (r : recover.Intended) (ht : f.target = .Cleared r) (hr : f.current ≠ .Wrote o) :
    rollback.decide_restore k f (.Obj o) ⦃ x => x = .Refuse (.Blocked k f.dest) ⦄ := by
  obtain ⟨dest, place, route, current, target, when⟩ := f
  simp only at ht hr
  subst ht
  cases r <;> cases place <;> cases current <;> restore_cases

/-- A write that no retained bytes reproduce exactly refuses the rollback:
an absent destination whose target record has no retained bytes. -/
theorem decide_restore_unrestorable_refused (k : U32) (f : rollback.UnitFacts) (o : recover.Ident)
    (ht : f.target = .Wrote o .Missing) :
    rollback.decide_restore k f .Absent ⦃ r => r = .Refuse (.Unrestorable k f.dest) ⦄ := by
  obtain ⟨dest, place, route, current, target, when⟩ := f
  simp only at ht
  subst ht
  cases place <;> cases current <;> restore_cases

/-- The decision never reports a directory step or a hold: those are the
directory items' own reports. -/
theorem decide_restore_not_directory (k : U32) (f : rollback.UnitFacts) (live : recover.Live) :
    rollback.decide_restore k f live ⦃ r => ∀ st, r = .Leave st → st ≠ .Directory ⦄ := by
  obtain ⟨dest, place, route, current, target, when⟩ := f
  cases target with
  | Released r => cases r <;> cases place <;> cases live <;> cases current <;> restore_cases
  | Preserved => cases place <;> cases live <;> cases current <;> restore_cases
  | Wrote o b => cases b <;> cases place <;> cases live <;> cases current <;> restore_cases
  | Cleared r => cases r <;> cases place <;> cases live <;> cases current <;> restore_cases

theorem lands_spec (i : recover.Intended) (l : recover.Live) :
    rollback.lands i l ⦃ b => b ↔ Lands i l ⦄ := by
  cases i <;> cases l <;> simp only [rollback.lands] <;> try simp [Lands]
  step with Ident.same_spec as ⟨ r, hr ⟩
  simp_all [eq_comm]

/-! ## Admission: a fresh transaction (`Fresh`, `fresh_spec` in Selection.lean), a complete target -/

/-- The plan the machine executes: the target retains its own profile, and
the selection is fresh. -/
def Admitted (p : rollback.PlanFacts) : Prop :=
  p.profile = .Retained ∧ Fresh p.reselect p.previous

theorem admit_spec (p : rollback.PlanFacts) :
    rollback.admit p ⦃ o => o = none → Admitted p ⦄ := by
  unfold rollback.admit
  cases hp : p.profile with
  | Missing => simp
  | Retained =>
    simp only
    step with fresh_spec as ⟨ b, hb ⟩
    split_ifs with h
    · simp only [WP.spec_ok]
      exact fun _ => ⟨hp, hb h⟩
    · simp

/-! ## What each step helper returns -/

/-! ## The directory sub-protocol's facts -/

/-- The stage a unit belongs to: a clear, or a unit the host settles before
the shape changes, comes in the clearing stage; every other unit after the
holds. -/
def UnitStage : rollback.Target → rollback.When → directory.Stage
  | .Cleared _, _ => .Clearing
  | _, .BeforeShape => .Clearing
  | _, .AfterShape => .Units

/-- A unit that would write the target's record or clear its destination. -/
def WritesTarget : rollback.Target → Prop
  | .Wrote _ _ => True
  | .Cleared _ => True
  | .Released _ => False
  | .Preserved => False

theorem unit_stage_ok (t : rollback.Target) (w : rollback.When) :
    rollback.unit_stage t w = ok (UnitStage t w) := by
  cases t <;> cases w <;> simp [rollback.unit_stage, rollback.clears, UnitStage]

theorem writes_target_spec (t : rollback.Target) :
    rollback.writes_target t ⦃ b => b ↔ WritesTarget t ⦄ := by
  cases t <;> simp [rollback.writes_target, WritesTarget]

/-- The claims after a unit is read: a unit that restores the target's
record claims its destination for the leaf protocol. -/
def ClaimsAfter (t : rollback.Target) (l : List ids.DestId) (d : ids.DestId) : List ids.DestId :=
  match t with
  | .Wrote _ _ => l ++ [d]
  | .Cleared _ => l
  | .Released _ => l
  | .Preserved => l

/-- The step ended the run: an abort, with the index and the plan as they
were. -/
def Aborts (s : rollback.RollbackState) (p : rollback.RollbackState × rollback.Command) : Prop :=
  ∃ r, p.2 = .Abort r ∧ p.1.index = s.index ∧ p.1.plan = s.plan ∧ p.1.phase = .Aborted

section helpers
variable (s : rollback.RollbackState)

theorem refuse_spec' (r : rollback.Refusal) :
    rollback.refuse s r ⦃ p => p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  simp [rollback.refuse]

theorem failed_or_protocol_spec' (e : rollback.Event) :
    rollback.failed_or_protocol s e ⦃ p =>
      ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold rollback.failed_or_protocol
  cases e <;> simp [rollback.refuse]

/-- Where `proceed` takes the run from `s`: the next unit, or the flip once
the index reached the plan's unit count. -/
def Proceed (p : rollback.RollbackState × rollback.Command) : Prop :=
  (s.index.val < s.plan.units.val ∧ p = ({ s with phase := .ReadingUnit }, .ReadUnit s.index)) ∨
  (s.plan.units.val ≤ s.index.val ∧ p = ({ s with phase := .Flipping }, .Flip s.plan.reselect))

theorem proceed_spec : rollback.proceed s ⦃ p => Proceed s p ⦄ := by
  unfold rollback.proceed Proceed
  split_ifs with h
  · simp only [WP.spec_ok]; left; exact ⟨by scalar_tac, by simp⟩
  · simp only [WP.spec_ok]; right; exact ⟨by scalar_tac, by simp⟩

/-- Where `advance` takes the run: the unit after `s.index`, or a refusal
when the counter would overflow. -/
def Advance (p : rollback.RollbackState × rollback.Command) : Prop :=
  p = ({ s with unit := none, install := none, step := none, phase := .Aborted },
    .Abort .Malformed) ∨
  ∃ i : U32, i.val = s.index.val + 1 ∧
    Proceed { s with index := i, unit := none, install := none, step := none } p

theorem advance_spec : rollback.advance s ⦃ p => Advance s p ⦄ := by
  unfold rollback.advance Advance
  have h := U32.checked_add_bv_spec s.index 1#u32
  cases ho : U32.checked_add s.index 1#u32 with
  | none => simp [lift, rollback.refuse]
  | some i =>
    simp only [ho] at h
    simp only [lift, Std.bind_ok]
    refine WP.spec_mono
      (proceed_spec { s with index := i, unit := none, install := none, step := none }) ?_
    intro p hp
    right
    exact ⟨i, by simpa using h.2.1, hp⟩

theorem report_spec (d : ids.DestId) (st : rollback.Settled) :
    rollback.report s d st ⦃ p => p = ({ s with phase := .Reporting }, .Report s.index d st) ⦄ := by
  simp [rollback.report]

theorem on_plan_spec (pl : rollback.PlanFacts) :
    rollback.on_plan s pl ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      (Admitted pl ∧ Proceed { s with plan := pl, index := 0#u32 } p) ⦄ := by
  unfold rollback.on_plan
  step with admit_spec as ⟨ o, ho ⟩
  cases o with
  | none =>
    simp only
    refine WP.spec_mono (proceed_spec _) ?_
    intro p hp
    exact Or.inr ⟨ho rfl, hp⟩
  | some r => simp [rollback.refuse]

/-- Where a unit read takes the run: an abort, or — in stage order, as the
unit's stage — a unit at a held destination reported held (only when it
would write nothing), or the unit examined off the held destinations. -/
def OnUnit (f : rollback.UnitFacts) (p : rollback.RollbackState × rollback.Command) : Prop :=
  Aborts s p ∨
  (Directory.Rank s.stage ≤ Directory.Rank (UnitStage f.target f.when) ∧ (
    (f.dest ∈ s.holds.val ∧ ¬ WritesTarget f.target ∧
      p = ({ s with unit := some f, stage := UnitStage f.target f.when, phase := .Reporting },
        .Report s.index f.dest .Held)) ∨
    (f.dest ∉ s.holds.val ∧ ∃ cl : alloc.vec.Vec ids.DestId,
      cl.val = ClaimsAfter f.target s.claimed.val f.dest ∧
      (p = ({ s with phase := .Observing, unit := some f, stage := UnitStage f.target f.when, claimed := cl }, .Observe s.index f.dest) ∨
       p = ({ s with phase := .Delegating, unit := some f, stage := UnitStage f.target f.when, claimed := cl }, .Delegate s.index f.dest)))))

theorem on_unit_spec (f : rollback.UnitFacts) :
    rollback.on_unit s f ⦃ p => OnUnit s f p ⦄ := by
  unfold rollback.on_unit OnUnit
  rw [unit_stage_ok, Std.bind_ok]
  obtain ⟨b, hb, hbs⟩ := spec_exists_ok (Directory.ordered_spec s.stage (UnitStage f.target f.when))
  rw [hb, Std.bind_ok]
  cases b
  · simp [rollback.refuse, Aborts]
  have hrank := hbs.mp rfl
  simp only [↓reduceIte]
  obtain ⟨b1, hb1, hh⟩ := spec_exists_ok (Directory.names_spec (alloc.vec.Vec.deref s.holds) f.dest)
  rw [hb1, Std.bind_ok]
  simp only [deref_val] at hh
  cases b1
  · have hnh : f.dest ∉ s.holds.val := fun h => by simpa using hh.mpr h
    simp only [Bool.false_eq_true, ↓reduceIte]
    cases ht : f.target <;>
      simp only [rollback.records_write, Std.bind_ok, Bool.false_eq_true, ↓reduceIte]
    case Wrote o bytes =>
      split_ifs with hmax
      · simp [rollback.refuse, Aborts]
      · have hpush : s.claimed.val.length < Usize.max := by
          have := alloc.vec.Vec.len_val s.claimed
          have hne : (alloc.vec.Vec.len s.claimed).val ≠ Usize.max := by
            intro e
            apply hmax
            exact UScalar.eq_of_val_eq (by simpa [core.num.Usize.MAX] using e)
          have := s.claimed.property
          simp only [alloc.vec.Vec.len_val] at hne
          scalar_tac
        apply WP.spec_bind (alloc.vec.Vec.push_spec _ _ hpush)
        intro v hv
        cases hr : f.route <;> simp only [WP.spec_ok] <;>
          exact Or.inr ⟨by simpa [ht] using hrank, Or.inr ⟨hnh, v, by simp [ClaimsAfter, hv]⟩⟩
    all_goals (cases hr : f.route <;> simp only [WP.spec_ok] <;>
      exact Or.inr ⟨by simpa [ht] using hrank, Or.inr ⟨hnh, s.claimed, by simp [ClaimsAfter]⟩⟩)
  · have hin : f.dest ∈ s.holds.val := hh.mp rfl
    simp only [↓reduceIte]
    obtain ⟨b2, hb2, hw⟩ := spec_exists_ok (writes_target_spec f.target)
    rw [hb2, Std.bind_ok]
    cases b2
    · simp only [Bool.false_eq_true, ↓reduceIte, rollback.report, WP.spec_ok]
      exact Or.inr ⟨hrank, Or.inl ⟨hin, fun h => by simpa using hw.mpr h, by simp⟩⟩
    · simp [rollback.refuse, Aborts]

/-- Where a directory item takes the run: an abort, or — in stage order, for
a destination no unit restores the target's record to — the commanded step,
or the recorded hold. -/
def OnDirectory (f : directory.DirectoryFacts) (p : rollback.RollbackState × rollback.Command) :
    Prop :=
  Aborts s p ∨
  (Directory.Rank s.stage ≤ Directory.Rank (Directory.StageOf f.action) ∧
    f.dest ∉ s.claimed.val ∧ (
    (Directory.Commands f.action ∧
      p = ({ s with phase := .Directing, stage := Directory.StageOf f.action, step := some f },
        .Directory s.index f.dest f.action)) ∨
    (¬ Directory.Commands f.action ∧ ∃ v : alloc.vec.Vec ids.DestId,
      v.val = s.holds.val ++ [f.dest] ∧
      p = ({ s with stage := Directory.StageOf f.action, holds := v, step := some f, phase := .Reporting }, .Report s.index f.dest .Held))))

theorem on_directory_spec (f : directory.DirectoryFacts) :
    rollback.on_directory s f ⦃ p => OnDirectory s f p ⦄ := by
  unfold rollback.on_directory OnDirectory
  rw [Directory.stage_ok, Std.bind_ok]
  obtain ⟨b, hb, hbs⟩ :=
    spec_exists_ok (Directory.ordered_spec s.stage (Directory.StageOf f.action))
  rw [hb, Std.bind_ok]
  cases b
  · simp [rollback.refuse, Aborts]
  have hord := hbs.mp rfl
  simp only [↓reduceIte]
  obtain ⟨b1, hb1, hb1s⟩ :=
    spec_exists_ok (Directory.names_spec (alloc.vec.Vec.deref s.claimed) f.dest)
  rw [hb1, Std.bind_ok]
  simp only [deref_val] at hb1s
  cases b1
  · have hcl : f.dest ∉ s.claimed.val := fun h => by simpa using hb1s.mpr h
    simp only [Bool.false_eq_true, ↓reduceIte]
    obtain ⟨b2, hb2, hb2s⟩ := spec_exists_ok (Directory.commands_spec f.action)
    rw [hb2, Std.bind_ok]
    cases b2
    · have hcmd : ¬ Directory.Commands f.action := fun h => by simpa using hb2s.mpr h
      simp only [Bool.false_eq_true, ↓reduceIte]
      split_ifs with hmax
      · simp [rollback.refuse, Aborts]
      · have hpush : s.holds.val.length < Usize.max := by
          have := alloc.vec.Vec.len_val s.holds
          have hne : (alloc.vec.Vec.len s.holds).val ≠ Usize.max := by
            intro e
            apply hmax
            exact UScalar.eq_of_val_eq (by simpa [core.num.Usize.MAX] using e)
          have := s.holds.property
          simp only [alloc.vec.Vec.len_val] at hne
          scalar_tac
        apply WP.spec_bind (alloc.vec.Vec.push_spec _ _ hpush)
        intro v hv
        simp only [rollback.report, WP.spec_ok]
        exact Or.inr ⟨hord, hcl, Or.inr ⟨hcmd, v, hv, by simp⟩⟩
    · simp only [↓reduceIte, WP.spec_ok]
      exact Or.inr ⟨hord, hcl, Or.inl ⟨hb2s.mp rfl, by simp⟩⟩
  · simp [rollback.refuse, Aborts]

theorem on_observed_spec (d : ids.DestId) (live : recover.Live) :
    rollback.on_observed s d live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f, s.unit = some f ∧ f.dest = d ∧
        ((∃ i, WriteOk f live i ∧
            p = ({ s with phase := .Writing, install := some i }, .Mutate s.index d live i)) ∨
         (∃ st, st ≠ .Restored ∧ st ≠ .Directory ∧
            p = ({ s with phase := .Reporting }, .Report s.index d st))) ⦄ := by
  unfold rollback.on_observed
  cases he : s.unit with
  | none => simp [rollback.refuse, he]
  | some f =>
    simp only
    step with DestId.same_spec as ⟨ b, hb ⟩
    split_ifs with hd
    · have hfd : f.dest = d := hb.mp hd
      have hspec := decide_restore_spec s.index f live
      obtain ⟨r, hr, hw, hra⟩ := spec_exists_ok hspec
      rw [hr, Std.bind_ok]
      cases r with
      | Write i =>
        simp only [WP.spec_ok]
        exact Or.inr ⟨f, rfl, hfd, Or.inl ⟨i, hw i rfl, by simp⟩⟩
      | Leave st =>
        have hnd : st ≠ .Directory := by
          obtain ⟨r', hr', hnd⟩ := spec_exists_ok (decide_restore_not_directory s.index f live)
          rw [hr] at hr'
          simp only [ok.injEq] at hr'
          subst hr'
          exact hnd st rfl
        simp only [rollback.report, WP.spec_ok]
        exact Or.inr ⟨f, rfl, hfd, Or.inr ⟨st, fun h => hra (by rw [h]), hnd, by simp [he]⟩⟩
      | Refuse r => simp [rollback.refuse, he]
    · simp [rollback.refuse, he]

theorem on_landed_spec (live : recover.Live) :
    rollback.on_landed s live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f i, s.unit = some f ∧ s.install = some i ∧ Lands i live ∧
        p = ({ s with phase := .Reporting }, .Report s.index f.dest .Restored) ⦄ := by
  unfold rollback.on_landed
  cases he : s.unit with
  | none => simp [rollback.refuse, he]
  | some f =>
    cases hi : s.install with
    | none => simp [rollback.refuse, he, hi]
    | some i =>
      simp only
      step with lands_spec as ⟨ b, hb ⟩
      split_ifs with h
      · simp only [rollback.report, WP.spec_ok]
        exact Or.inr ⟨f, i, rfl, rfl, hb.mp h, by simp [he, hi]⟩
      · simp [rollback.refuse, he, hi]

theorem on_write_failed_spec (failure : recover.Failure) :
    rollback.on_write_failed s failure ⦃ p =>
      ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold rollback.on_write_failed
  cases failure <;> cases he : s.unit <;> simp [rollback.refuse, he]

theorem on_delegated_spec (e : rollback.Event) :
    rollback.on_delegated s e ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f, s.unit = some f ∧ e = .Done ∧
        p = ({ s with phase := .Reporting }, .Report s.index f.dest .Delegated) ⦄ := by
  unfold rollback.on_delegated
  cases he : s.unit with
  | none => simp [rollback.refuse, he]
  | some f =>
    cases e <;> simp [rollback.failed_or_protocol, rollback.refuse, rollback.report, he]

/-- A commanded directory step is answered: settled and reported as such, or
refused, which aborts. -/
theorem on_directed_spec (e : rollback.Event) :
    rollback.on_directed s e ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f, s.step = some f ∧ e = .Done ∧
        p = ({ s with phase := .Reporting }, .Report s.index f.dest .Directory) ⦄ := by
  unfold rollback.on_directed
  cases he : s.step with
  | none => simp [rollback.refuse, he]
  | some f =>
    cases e <;> simp [rollback.failed_or_protocol, rollback.refuse, rollback.report, he]

end helpers

/-! ## The invariant of every run -/

/-- Phases after the plan was admitted. -/
def Planned (p : rollback.Phase) : Prop :=
  p = .ReadingUnit ∨ p = .Observing ∨ p = .Writing ∨ p = .Delegating ∨ p = .Directing ∨
  p = .Reporting ∨ p = .Flipping ∨ p = .Finished

/-- A unit is under examination (observed, written or delegated). -/
def Examining (p : rollback.Phase) : Prop :=
  p = .Observing ∨ p = .Writing ∨ p = .Delegating

/-- Unit `k` was reported settled in `cs`. -/
def Reported (k : U32) (cs : List rollback.Command) : Prop :=
  ∃ d st, rollback.Command.Report k d st ∈ cs

theorem Reported.mono {k : U32} {cs : List rollback.Command} (h : Reported k cs)
    (c : rollback.Command) : Reported k (cs ++ [c]) := by
  obtain ⟨d, st, h⟩ := h
  exact ⟨d, st, List.mem_append_left _ h⟩

/-- Every directory step commanded in `cs` was reported settled. -/
def DirectoriesSettled (cs : List rollback.Command) : Prop :=
  ∀ k d a, rollback.Command.Directory k d a ∈ cs → rollback.Command.Report k d .Directory ∈ cs

structure Inv (s : rollback.RollbackState) (cs : List rollback.Command) : Prop where
  /-- Every unit before the one under decision was reported. -/
  reported : ∀ k : U32, k.val < s.index.val → Reported k cs
  /-- While reporting, the unit under decision was just reported. -/
  reporting : s.phase = .Reporting → Reported s.index cs
  /-- After the plan was read, it was admitted. -/
  admitted : Planned s.phase → Admitted s.plan
  /-- While writing, the outstanding write installs the unit's target. -/
  writing : s.phase = .Writing → ∃ f i, s.unit = some f ∧ s.install = some i ∧ Installs f.target i
  /-- A unit under examination is not at a destination the directory domain
  holds. -/
  unheld : Examining s.phase → ∀ f, s.unit = some f → f.dest ∉ s.holds.val
  /-- Every directory step commanded was reported settled — except the one
  outstanding, or after an abort. -/
  directed : ∀ k d a, rollback.Command.Directory k d a ∈ cs →
    rollback.Command.Report k d .Directory ∈ cs ∨ s.phase = .Aborted ∨
    (s.phase = .Directing ∧ k = s.index ∧ ∃ f, s.step = some f ∧ f.dest = d)

/-- The executions of the machine: from `RollbackState::new`, any sequence of
events the step accepts, with the commands it issued, in order. -/
inductive Run : rollback.RollbackState → List rollback.Command → Prop where
  | start {s} : rollback.RollbackState.new = ok s → Run s []
  | step {s cs e s' c} : Run s cs → rollback.rollback_step s e = ok (s', c) → Run s' (cs ++ [c])

/-- A command that is neither a write, a restored report, a directory step,
its settled report, nor the flip. -/
def Quiet (c : rollback.Command) : Prop :=
  (∀ u d x i, c ≠ .Mutate u d x i) ∧ (∀ u d, c ≠ .Report u d .Restored) ∧
    (∀ u d a, c ≠ .Directory u d a) ∧ (∀ u d, c ≠ .Report u d .Directory) ∧
    (∀ sel, c ≠ .Flip sel)

/-- What one step guarantees, given the invariant before it. -/
structure StepPost (s : rollback.RollbackState) (e : rollback.Event) (cs : List rollback.Command)
    (p : rollback.RollbackState × rollback.Command) : Prop where
  inv : Inv p.1 (cs ++ [p.2])
  mutate : ∀ u d x i, p.2 = .Mutate u d x i →
    s.phase = .Observing ∧ ∃ f, s.unit = some f ∧ e = .Observed d x ∧ f.dest = d ∧
      u = s.index ∧ WriteOk f x i ∧ d ∉ s.holds.val
  restored : ∀ u d, p.2 = .Report u d .Restored →
    s.phase = .Writing ∧ ∃ f i l, s.unit = some f ∧ s.install = some i ∧ Installs f.target i ∧
      e = .Landed l ∧ Lands i l
  directory : ∀ u d a, p.2 = .Directory u d a →
    s.phase = .ReadingUnit ∧ e = .DirectoryRead ⟨d, a⟩ ∧ u = s.index ∧ d ∉ s.claimed.val ∧
      Directory.Commands a ∧ Directory.Rank s.stage ≤ Directory.Rank (Directory.StageOf a)
  settled : ∀ u d, p.2 = .Report u d .Directory →
    s.phase = .Directing ∧ e = .Done ∧ u = s.index ∧ ∃ f, s.step = some f ∧ f.dest = d
  flip : ∀ sel, p.2 = .Flip sel →
    sel = p.1.plan.reselect ∧ Admitted p.1.plan ∧
      (∀ k : U32, k.val < p.1.plan.units.val → Reported k cs) ∧ DirectoriesSettled cs

theorem StepPost.of_quiet {s e cs p} (hi : Inv p.1 (cs ++ [p.2])) (hq : Quiet p.2) :
    StepPost s e cs p :=
  ⟨hi, fun u d x i h => absurd h (hq.1 u d x i), fun u d h => absurd h (hq.2.1 u d),
    fun u d a h => absurd h (hq.2.2.1 u d a), fun u d h => absurd h (hq.2.2.2.1 u d),
    fun sel h => absurd h (hq.2.2.2.2 sel)⟩

/-- The directory steps commanded in `cs ++ [c]`, for a `c` that commands none. -/
theorem directory_append {cs : List rollback.Command} {c : rollback.Command}
    (hc : ∀ k d a, c ≠ .Directory k d a) {k d a}
    (h : rollback.Command.Directory k d a ∈ cs ++ [c]) : rollback.Command.Directory k d a ∈ cs := by
  rcases List.mem_append.mp h with h | h
  · exact h
  · simp only [List.mem_singleton] at h
    exact absurd h.symm (hc k d a)

/-- Directory steps are settled whenever no step is outstanding and the run
did not abort. -/
theorem Inv.settled_now {s : rollback.RollbackState} {cs : List rollback.Command} (h : Inv s cs)
    (h1 : s.phase ≠ .Directing) (h2 : s.phase ≠ .Aborted) : DirectoriesSettled cs := by
  intro k d a hm
  rcases h.directed k d a hm with hr | ha | ⟨hd, -⟩
  · exact hr
  · exact absurd ha h2
  · exact absurd hd h1

/-- A step that keeps the index and plan, issues no directory step, and
moves to a phase that is not reporting, writing, examining or directing —
from one with no step outstanding, or into an abort — keeps the invariant. -/
theorem Inv.quiet {s : rollback.RollbackState} {cs : List rollback.Command} (h : Inv s cs)
    (s' : rollback.RollbackState) (c : rollback.Command)
    (hi : s'.index = s.index) (hp : s'.plan = s.plan)
    (h1 : s'.phase ≠ .Reporting) (h2 : s'.phase ≠ .Writing) (h3 : ¬ Examining s'.phase)
    (h4 : Planned s'.phase → Planned s.phase)
    (hd : s'.phase = .Aborted ∨ (s.phase ≠ .Directing ∧ s.phase ≠ .Aborted ∧ s'.phase ≠ .Directing))
    (hc : ∀ k d a, c ≠ .Directory k d a) : Inv s' (cs ++ [c]) where
  reported := fun k hk => (h.reported k (hi ▸ hk)).mono c
  reporting := fun hr => absurd hr h1
  admitted := fun hpl => hp ▸ h.admitted (h4 hpl)
  writing := fun hw => absurd hw h2
  unheld := fun he => absurd he h3
  directed := fun k d a hmem => by
    have hm' := directory_append hc hmem
    rcases hd with ha | ⟨hn1, hn2, -⟩
    · exact Or.inr (Or.inl ha)
    · exact Or.inl (List.mem_append_left _ (h.settled_now hn1 hn2 k d a hm'))

theorem post_abort {s : rollback.RollbackState} {e cs} (h : Inv s cs)
    (s' : rollback.RollbackState) (r : rollback.Refusal)
    (hi : s'.index = s.index) (hp : s'.plan = s.plan) (ha : s'.phase = .Aborted) :
    StepPost s e cs (s', .Abort r) :=
  StepPost.of_quiet (h.quiet _ _ hi hp (by simp [ha]) (by simp [ha]) (by simp [Examining, ha])
    (fun hpl => by simp [Planned, ha] at hpl) (Or.inl ha) (by simp)) (by simp [Quiet])

theorem post_aborts {s : rollback.RollbackState} {e cs p} (h : Inv s cs) (ha : Aborts s p) :
    StepPost s e cs p := by
  obtain ⟨r, h2, hi, hp, hph⟩ := ha
  obtain ⟨s', c⟩ := p
  simp only at h2 hi hp hph
  subst h2
  exact post_abort h s' r hi hp hph

/-- Entering `proceed` at index `t.index` with every earlier unit reported,
every directory step settled, and an admitted plan. -/
theorem post_proceed {s : rollback.RollbackState} {e cs} {t : rollback.RollbackState}
    {p : rollback.RollbackState × rollback.Command} (hp : Proceed t p)
    (hrep : ∀ k : U32, k.val < t.index.val → Reported k cs) (hadm : Admitted t.plan)
    (hdir : DirectoriesSettled cs) :
    StepPost s e cs p := by
  have hdir' : ∀ c : rollback.Command, (∀ k d a, c ≠ .Directory k d a) →
      ∀ k d a, rollback.Command.Directory k d a ∈ cs ++ [c] →
        rollback.Command.Report k d .Directory ∈ cs ++ [c] ∨ False ∨ False :=
    fun c hc k d a hm => Or.inl (List.mem_append_left _ (hdir k d a (directory_append hc hm)))
  rcases hp with ⟨hlt, rfl⟩ | ⟨hle, rfl⟩
  · refine StepPost.of_quiet ⟨fun k hk => (hrep k hk).mono _, by simp, fun _ => hadm, by simp,
      by simp [Examining], ?_⟩ (by simp [Quiet])
    intro k d a hm
    rcases hdir' _ (by simp) k d a hm with h | h | h
    · exact Or.inl h
    · exact h.elim
    · exact h.elim
  · refine ⟨⟨fun k hk => (hrep k hk).mono _, by simp, fun _ => hadm, by simp, by simp [Examining],
      ?_⟩, by simp, by simp, by simp, by simp, ?_⟩
    · intro k d a hm
      rcases hdir' _ (by simp) k d a hm with h | h | h
      · exact Or.inl h
      · exact h.elim
      · exact h.elim
    · intro sel hsel
      simp only [rollback.Command.Flip.injEq] at hsel
      exact ⟨hsel.symm, hadm, fun k hk => hrep k (by simp only at hk ⊢; omega), hdir⟩

/-- A step into reporting a unit (not restored, not a directory step) that
keeps the index and plan, from a planned phase with no step outstanding. -/
theorem Inv.report {s : rollback.RollbackState} {cs : List rollback.Command} (h : Inv s cs)
    (s' : rollback.RollbackState) (d : ids.DestId) (st : rollback.Settled)
    (hi : s'.index = s.index) (hp : s'.plan = s.plan) (hph : s'.phase = .Reporting)
    (hpl : Planned s.phase) (hs : s.phase ≠ .Directing ∧ s.phase ≠ .Aborted) :
    Inv s' (cs ++ [.Report s.index d st]) where
  reported := fun k hk => (h.reported k (hi ▸ hk)).mono _
  reporting := fun _ => ⟨d, st, by simp [hi]⟩
  admitted := fun _ => hp ▸ h.admitted hpl
  writing := fun hw => by simp [hph] at hw
  unheld := fun he => by simp [Examining, hph] at he
  directed := fun k d' a hmem => by
    have hm' := directory_append (by simp) hmem
    exact Or.inl (List.mem_append_left _ (h.settled_now hs.1 hs.2 k d' a hm'))

theorem step_spec (s : rollback.RollbackState) (e : rollback.Event) (cs : List rollback.Command)
    (h : Inv s cs) : rollback.rollback_step s e ⦃ p => StepPost s e cs p ⦄ := by
  rcases s with ⟨phase, plan, index, unit, install, stage, holds, claimed, step⟩
  unfold rollback.rollback_step
  cases phase <;> cases e
  all_goals try first
    | (with_reducible exact WP.spec_mono (failed_or_protocol_spec' _ _) (by rintro p ⟨r, rfl⟩; exact post_abort h _ r rfl rfl rfl))
    | (with_reducible exact WP.spec_mono (refuse_spec' _ _) (by rintro p rfl; exact post_abort h _ _ rfl rfl rfl))
  case Start.Start =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (by simp [Planned]) (Or.inr (by simp)) (by simp)) (by simp [Quiet])
  case ReadingPlan.PlanRead pl =>
    refine WP.spec_mono (on_plan_spec _ pl) ?_
    rintro p (⟨r, rfl⟩ | ⟨hadm, hp⟩)
    · exact post_abort h _ r rfl rfl rfl
    · exact post_proceed hp (fun k hk => by simp at hk) hadm (h.settled_now (by simp) (by simp))
  case ReadingUnit.UnitRead f =>
    refine WP.spec_mono (on_unit_spec _ f) ?_
    rintro p (ha | ⟨-, ⟨hh, hw, rfl⟩ | ⟨hh, cl, -, rfl | rfl⟩⟩)
    · exact post_aborts h ha
    · exact StepPost.of_quiet (h.report _ f.dest .Held rfl rfl rfl (by simp [Planned]) (by simp))
        (by simp [Quiet])
    all_goals
      refine StepPost.of_quiet ⟨fun k hk => (h.reported k hk).mono _, by simp,
        fun _ => h.admitted (by simp [Planned]), by simp, ?_, ?_⟩ (by simp [Quiet])
      · intro _ f' hf'
        simp only [Option.some.injEq] at hf'
        subst hf'
        exact hh
      · intro k d a hmem
        exact Or.inl (List.mem_append_left _
          (h.settled_now (by simp) (by simp) k d a (directory_append (by simp) hmem)))
  case ReadingUnit.DirectoryRead f =>
    refine WP.spec_mono (on_directory_spec _ f) ?_
    rintro p (ha | ⟨hord, hcl, ⟨hcmd, rfl⟩ | ⟨hcmd, v, hv, rfl⟩⟩)
    · exact post_aborts h ha
    · refine ⟨⟨fun k hk => (h.reported k hk).mono _, by simp,
          fun _ => h.admitted (by simp [Planned]), by simp, by simp [Examining], ?_⟩,
        by simp, by simp, ?_, by simp, by simp⟩
      · intro k d a hmem
        rcases List.mem_append.mp hmem with hm' | hm'
        · exact Or.inl (List.mem_append_left _ (h.settled_now (by simp) (by simp) k d a hm'))
        · simp only [List.mem_singleton, rollback.Command.Directory.injEq] at hm'
          obtain ⟨rfl, rfl, rfl⟩ := hm'
          exact Or.inr (Or.inr ⟨rfl, rfl, f, rfl, rfl⟩)
      · intro u d a hc
        simp only [rollback.Command.Directory.injEq] at hc
        obtain ⟨rfl, rfl, rfl⟩ := hc
        exact ⟨rfl, rfl, rfl, hcl, hcmd, hord⟩
    · exact StepPost.of_quiet (h.report _ f.dest .Held rfl rfl rfl (by simp [Planned]) (by simp))
        (by simp [Quiet])
  case Observing.Observed d live =>
    refine WP.spec_mono (on_observed_spec _ d live) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, hf, hfd, ⟨i, hw, rfl⟩ | ⟨st, hst, hsd, rfl⟩⟩)
    · exact post_abort h _ r rfl rfl rfl
    · have hh := h.unheld (by simp [Examining]) f hf
      refine ⟨⟨fun k hk => (h.reported k hk).mono _, by simp,
        fun _ => h.admitted (by simp [Planned]), fun _ => ⟨f, i, by simpa using hf, rfl, hw.1⟩,
        ?_, ?_⟩, ?_, by simp, by simp, by simp, by simp⟩
      · intro _ f' hf'
        simp only at hf hf'
        rw [hf] at hf'
        cases hf'
        exact hh
      · intro k d' a hmem
        exact Or.inl (List.mem_append_left _
          (h.settled_now (by simp) (by simp) k d' a (directory_append (by simp) hmem)))
      · intro u d' x i' hm
        simp only [rollback.Command.Mutate.injEq] at hm
        obtain ⟨rfl, rfl, rfl, rfl⟩ := hm
        exact ⟨rfl, f, hf, rfl, hfd, rfl, hw, hfd ▸ hh⟩
    · refine StepPost.of_quiet (h.report _ d st rfl rfl rfl (by simp [Planned]) (by simp))
        ⟨by simp, ?_, by simp, ?_, by simp⟩
      · intro u d' hr
        simp only [rollback.Command.Report.injEq] at hr
        exact hst hr.2.2
      · intro u d' hr
        simp only [rollback.Command.Report.injEq] at hr
        exact hsd hr.2.2
  case Writing.Landed live =>
    refine WP.spec_mono (on_landed_spec _ live) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, i, hf, hi, hl, rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    · obtain ⟨f', i', hf', hi', hins⟩ := h.writing rfl
      simp only at hf hi hf' hi'
      rw [hf] at hf'
      rw [hi] at hi'
      cases hf'
      cases hi'
      refine ⟨h.report _ f.dest .Restored rfl rfl rfl (by simp [Planned]) (by simp),
        by simp, ?_, by simp, by simp, by simp⟩
      intro u d hr
      exact ⟨rfl, f, i, live, hf, hi, hins, rfl, hl⟩
  case Writing.Failed failure =>
    exact WP.spec_mono (on_write_failed_spec _ failure)
      (by rintro p ⟨r, rfl⟩; exact post_abort h _ r rfl rfl rfl)
  case Reporting.Done =>
    refine WP.spec_mono (advance_spec _) ?_
    rintro p (rfl | ⟨i, hi, hp⟩)
    · exact post_abort h _ _ rfl rfl rfl
    · refine post_proceed hp (fun k hk => ?_) (h.admitted (by simp [Planned]))
        (h.settled_now (by simp) (by simp))
      simp only at hk hi
      by_cases hlt : k.val < index.val
      · exact h.reported k hlt
      · have : k = index := UScalar.eq_of_val_eq (by omega)
        subst this
        exact h.reporting rfl
  case Flipping.Done =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (fun _ => by simp [Planned]) (Or.inr (by simp)) (by simp)) (by simp [Quiet])
  all_goals first
    | (refine WP.spec_mono (on_delegated_spec _ _) ?_
       rintro p (⟨r, rfl⟩ | ⟨f, hf, -, rfl⟩)
       · exact post_abort h _ r rfl rfl rfl
       · exact StepPost.of_quiet (h.report _ f.dest .Delegated rfl rfl rfl (by simp [Planned])
           (by simp)) (by simp [Quiet]))
    | (refine WP.spec_mono (on_directed_spec _ _) ?_
       rintro p (⟨r, rfl⟩ | ⟨f, hf, he, rfl⟩)
       · exact post_abort h _ r rfl rfl rfl
       · refine ⟨⟨fun k hk => (h.reported k hk).mono _, fun _ => ⟨f.dest, .Directory, by simp⟩,
             fun _ => h.admitted (by simp [Planned]), by simp, by simp [Examining], ?_⟩,
           by simp, by simp, by simp, ?_, by simp⟩
         · intro k d a hmem
           rcases List.mem_append.mp hmem with hm' | hm'
           · rcases h.directed k d a hm' with hr | ha | ⟨-, hk, f', hf', hd⟩
             · exact Or.inl (List.mem_append_left _ hr)
             · simp at ha
             · simp only at hf hf'
               rw [hf] at hf'
               cases hf'
               subst hk hd
               exact Or.inl (by simp)
           · simp at hm'
         · intro u d hc
           simp only [rollback.Command.Report.injEq] at hc
           obtain ⟨rfl, rfl, -⟩ := hc
           exact ⟨rfl, he, rfl, f, hf, rfl⟩)

/-! ## The theorems -/

/-- Every execution keeps the invariant. -/
theorem run_inv {s : rollback.RollbackState} {cs : List rollback.Command} (h : Run s cs) :
    Inv s cs := by
  induction h with
  | start hs =>
    simp only [rollback.RollbackState.new, generation.GenerationId.new, Std.bind_ok,
      ok.injEq] at hs
    subst hs
    exact ⟨fun k hk => by simp at hk, by simp, fun h => by simp [Planned] at h, by simp,
      by simp [Examining], fun k d a hm => by simp at hm⟩
  | step _ hstep ih => exact (spec_eq_ok (step_spec _ _ _ ih) hstep).inv

theorem run_post {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {c : rollback.Command} (hrun : Run s cs)
    (h : rollback.rollback_step s e = ok (s', c)) : StepPost s e cs (s', c) :=
  spec_eq_ok (step_spec _ _ _ (run_inv hrun)) h

/-- (K2) Rollback fidelity. In every execution, a write is issued only in
answer to the observation of the unit under decision, names that unit and its
destination, carries the observed object as `expect` (the driver re-checks it
before acting), and installs exactly the target generation's recorded
identity for that unit, from retained bytes — or removes gripsack's write
where the target deploys nothing, or a shape change needs it gone. Nothing
else is ever written. -/
theorem rollback_installs_target {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Mutate u d x i)) :
    s.phase = .Observing ∧ ∃ f, s.unit = some f ∧ e = .Observed d x ∧ f.dest = d ∧
      u = s.index ∧ Installs f.target i := by
  obtain ⟨hph, f, hf, he, hd, hu, hw, -⟩ := (run_post hrun h).mutate _ _ _ _ rfl
  exact ⟨hph, f, hf, he, hd, hu, hw.1⟩

/-- (K1 through rollback) In every execution, a write replaces only absence
or gripsack's own write in the current generation: drift is never
overwritten, and rollback never assumes consent. -/
theorem rollback_write_k1 {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Mutate u d x i)) :
    ∃ f, s.unit = some f ∧ (x = .Absent ∨ Ours f.current x) := by
  obtain ⟨-, f, hf, -, -, -, hw, -⟩ := (run_post hrun h).mutate _ _ _ _ rfl
  exact ⟨f, hf, hw.2.2⟩

/-- In every execution, no write targets a destination inside a published
generation's directory (FND-2's backfill was such a write). The flip, the
only other effect, names a selection. -/
theorem rollback_never_writes_generation {s s' : rollback.RollbackState}
    {cs : List rollback.Command} {e : rollback.Event} {u : U32} {d : ids.DestId}
    {x : recover.Live} {i : recover.Intended}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Mutate u d x i)) :
    ∃ f, s.unit = some f ∧ f.dest = d ∧ f.place = .Outside := by
  obtain ⟨-, f, hf, -, hd, -, hw, -⟩ := (run_post hrun h).mutate _ _ _ _ rfl
  exact ⟨f, hf, hd, hw.2.1⟩

/-- (Directory fence) In every execution, no write targets a destination the
directory domain holds (a node of a target receipt). -/
theorem rollback_write_unheld {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Mutate u d x i)) :
    d ∉ s.holds.val := by
  obtain ⟨-, -, -, -, -, -, -, hh⟩ := (run_post hrun h).mutate _ _ _ _ rfl
  exact hh

/-- (Directory fence) In every execution, a directory step is commanded only
when the host handed over that directory item, for a destination no unit of
the rollback restores the target's record to, for an action the host
executes (never a hold), and in stage order. -/
theorem rollback_directory_unclaimed {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {u : U32} {d : ids.DestId} {a : directory.Action}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Directory u d a)) :
    e = .DirectoryRead ⟨d, a⟩ ∧ u = s.index ∧ d ∉ s.claimed.val ∧ Directory.Commands a ∧
      Directory.Rank s.stage ≤ Directory.Rank (Directory.StageOf a) := by
  obtain ⟨-, he, hu, hc, hcmd, hr⟩ := (run_post hrun h).directory _ _ _ rfl
  exact ⟨he, hu, hc, hcmd, hr⟩

/-- (Directory fence) In every execution, a directory step is reported
settled only in answer to the host's settlement of the outstanding step. -/
theorem rollback_directory_settled_answers {s s' : rollback.RollbackState}
    {cs : List rollback.Command} {e : rollback.Event} {u : U32} {d : ids.DestId}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Report u d .Directory)) :
    s.phase = .Directing ∧ e = .Done ∧ u = s.index ∧ ∃ f, s.step = some f ∧ f.dest = d :=
  (run_post hrun h).settled _ _ rfl

/-- A directory step the host cannot settle ends the rollback: the step
answers `Abort` naming the item, and the machine is aborted. -/
theorem rollback_directory_refusal_aborts (s : rollback.RollbackState)
    (f : directory.DirectoryFacts) (hph : s.phase = .Directing) (hs : s.step = some f) :
    rollback.rollback_step s .Refused ⦃ p =>
      p.2 = .Abort (.Directory s.index f.dest) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, plan, index, unit, install, stage, holds, claimed, step⟩
  simp only at hph hs
  subst hph hs
  simp [rollback.rollback_step, rollback.on_directed, rollback.refuse]

/-- No unit of an earlier stage than the rollback's is accepted: it aborts. -/
theorem rollback_unit_out_of_stage_aborts (s : rollback.RollbackState) (f : rollback.UnitFacts)
    (hph : s.phase = .ReadingUnit)
    (hr : Directory.Rank (UnitStage f.target f.when) < Directory.Rank s.stage) :
    rollback.rollback_step s (.UnitRead f) ⦃ p =>
      p.2 = .Abort (.Order s.index) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, plan, index, unit, install, stage, holds, claimed, step⟩
  simp only at hph hr
  subst hph
  unfold rollback.rollback_step rollback.on_unit
  simp only
  rw [unit_stage_ok, Std.bind_ok]
  obtain ⟨b, hb, hb'⟩ := spec_exists_ok (Directory.ordered_spec stage (UnitStage f.target f.when))
  rw [hb, Std.bind_ok]
  have : b = false := by
    cases b
    · rfl
    · exact absurd (hb'.mp rfl) (by omega)
  simp [this, rollback.refuse]

/-- No directory item of an earlier stage than the rollback's is accepted:
it aborts, so no step is commanded out of order. -/
theorem rollback_directory_out_of_stage_aborts (s : rollback.RollbackState)
    (f : directory.DirectoryFacts) (hph : s.phase = .ReadingUnit)
    (hr : Directory.Rank (Directory.StageOf f.action) < Directory.Rank s.stage) :
    rollback.rollback_step s (.DirectoryRead f) ⦃ p =>
      p.2 = .Abort (.Order s.index) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, plan, index, unit, install, stage, holds, claimed, step⟩
  simp only at hph hr
  subst hph
  unfold rollback.rollback_step rollback.on_directory
  simp only
  rw [Directory.stage_ok, Std.bind_ok]
  obtain ⟨b, hb, hb'⟩ := spec_exists_ok (Directory.ordered_spec stage (Directory.StageOf f.action))
  rw [hb, Std.bind_ok]
  have : b = false := by
    cases b
    · rfl
    · exact absurd (hb'.mp rfl) (by omega)
  simp [this, rollback.refuse]

/-- In every execution, a unit is reported restored only when the object
that landed is exactly what the write installed — the target's recorded
identity (or the removal). -/
theorem rollback_restored_landed {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {u : U32} {d : ids.DestId}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Report u d .Restored)) :
    s.phase = .Writing ∧ ∃ f i l, s.unit = some f ∧ s.install = some i ∧ Installs f.target i ∧
      e = .Landed l ∧ Lands i l :=
  (run_post hrun h).restored _ _ rfl

/-- In every execution, the flip is issued only after every unit of the plan
was reported settled — restored, already there, kept, untouched, settled by
the host's own domain, held, or a settled directory step —, after every
directory step commanded was reported settled by the host, and names the
planned selection. An unrestorable unit or a refused directory step aborts
the run, so it never reaches the flip. -/
theorem rollback_flip_after_every_unit {s s' : rollback.RollbackState}
    {cs : List rollback.Command} {e : rollback.Event} {sel : selection.SelectionIdentity}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Flip sel)) :
    sel = s'.plan.reselect ∧ (∀ k : U32, k.val < s'.plan.units.val → Reported k cs) ∧
      DirectoriesSettled cs := by
  obtain ⟨hs, -, hk, hd⟩ := (run_post hrun h).flip _ rfl
  exact ⟨hs, hk, hd⟩

/-- (K6) In every execution, the flip names a transaction selection whose
transaction differs from the one it replaces, and the target retains its own
shell profile: rollback reselects, it never writes into a generation. -/
theorem rollback_flip_admitted {s s' : rollback.RollbackState}
    {cs : List rollback.Command} {e : rollback.Event} {sel : selection.SelectionIdentity}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Flip sel)) :
    s'.plan.profile = .Retained ∧ Fresh sel s'.plan.previous := by
  obtain ⟨hs, ⟨hp, hf⟩, -⟩ := (run_post hrun h).flip _ rfl
  exact ⟨hp, hs ▸ hf⟩

/-- R3: a unit whose target record no retained bytes reproduce, observed
absent, aborts the run naming the unit — it is never reported settled, so
the flip never follows. -/
theorem rollback_unrestorable_aborts (s : rollback.RollbackState) (f : rollback.UnitFacts)
    (o : recover.Ident) (hph : s.phase = .Observing) (hu : s.unit = some f)
    (ht : f.target = .Wrote o .Missing) :
    rollback.rollback_step s (.Observed f.dest .Absent) ⦃ p =>
      p.2 = .Abort (.Unrestorable s.index f.dest) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, plan, index, unit, install, stage, holds, claimed, step⟩
  simp only at hph hu
  subst hph hu
  unfold rollback.rollback_step rollback.on_observed
  simp only
  obtain ⟨b, hb, hb'⟩ := spec_exists_ok (DestId.same_spec f.dest f.dest)
  have htrue : b = true := hb'.mpr rfl
  rw [hb, Std.bind_ok]
  simp only [htrue, ↓reduceIte]
  obtain ⟨r, hr, hr'⟩ := spec_exists_ok (decide_restore_unrestorable_refused index f o ht)
  rw [hr, Std.bind_ok, hr']
  simp [rollback.refuse]

/-- R3: a unit the host's own domain cannot restore (a managed block whose
host file is no longer text) aborts the run naming the unit. -/
theorem rollback_delegated_refusal_aborts (s : rollback.RollbackState) (f : rollback.UnitFacts)
    (hph : s.phase = .Delegating) (hu : s.unit = some f) :
    rollback.rollback_step s .Refused ⦃ p =>
      p.2 = .Abort (.Unrestorable s.index f.dest) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, plan, index, unit, install, stage, holds, claimed, step⟩
  simp only at hph hu
  subst hph hu
  simp [rollback.rollback_step, rollback.on_delegated, rollback.refuse]

end Gripsack.Kernel.Rollback
