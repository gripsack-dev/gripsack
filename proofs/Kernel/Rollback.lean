import Extracted.GripsackKernel
import Kernel.Selection
import Kernel.Recover
import Kernel.Ownership

/-!
# The rollback machine (plan/0056 F2 step 8)

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
  kept, untouched or delegated), and names the planned selection.
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
    rollback.withheld, rollback.retained, rollback.writable, ownership.decide_unit, ownership.Recorded.wrote, ownership.acquires,
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
  obtain ⟨dest, place, route, current, target⟩ := f
  cases target with
  | Released r => cases r <;> cases place <;> cases live <;> cases current <;> restore_cases
  | Preserved => cases place <;> cases live <;> cases current <;> restore_cases
  | Wrote o b => cases b <;> cases place <;> cases live <;> cases current <;> restore_cases

/-- K1 through rollback: a live object that is not gripsack's current write
and is not already what the target records is kept, never written (a release
is authorized as a removal, so only gripsack's own write is ever released). -/
theorem decide_restore_drift_kept (k : U32) (f : rollback.UnitFacts) (o : recover.Ident)
    (hr : f.current ≠ .Wrote o) (ht : ∀ b, f.target ≠ .Wrote o b) :
    rollback.decide_restore k f (.Obj o) ⦃ r =>
      (∃ why, r = .Leave (.Kept why)) ∨ r = .Leave .Untouched ⦄ := by
  obtain ⟨dest, place, route, current, target⟩ := f
  simp only at hr ht
  cases target with
  | Released r => cases r <;> cases place <;> cases current <;> restore_cases
  | Preserved => cases place <;> cases current <;> restore_cases
  | Wrote t b =>
    have hto : t ≠ o := fun h => ht b (by rw [h])
    cases b <;> cases place <;> cases current <;> restore_cases

/-- A write that no retained bytes reproduce exactly refuses the rollback:
an absent destination whose target record has no retained bytes. -/
theorem decide_restore_unrestorable_refused (k : U32) (f : rollback.UnitFacts) (o : recover.Ident)
    (ht : f.target = .Wrote o .Missing) :
    rollback.decide_restore k f .Absent ⦃ r => r = .Refuse (.Unrestorable k f.dest) ⦄ := by
  obtain ⟨dest, place, route, current, target⟩ := f
  simp only at ht
  subst ht
  cases place <;> cases current <;> restore_cases

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
  p = ({ s with unit := none, install := none, phase := .Aborted }, .Abort .Malformed) ∨
  ∃ i : U32, i.val = s.index.val + 1 ∧ Proceed { s with index := i, unit := none, install := none } p

theorem advance_spec : rollback.advance s ⦃ p => Advance s p ⦄ := by
  unfold rollback.advance Advance
  have h := U32.checked_add_bv_spec s.index 1#u32
  cases ho : U32.checked_add s.index 1#u32 with
  | none => simp [lift, rollback.refuse]
  | some i =>
    simp only [ho] at h
    simp only [lift, Std.bind_ok]
    refine WP.spec_mono (proceed_spec { s with index := i, unit := none, install := none }) ?_
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

theorem on_unit_spec (f : rollback.UnitFacts) :
    rollback.on_unit s f ⦃ p =>
      p = ({ s with phase := .Observing, unit := some f }, .Observe s.index f.dest) ∨
      p = ({ s with phase := .Delegating, unit := some f }, .Delegate s.index f.dest) ⦄ := by
  unfold rollback.on_unit
  cases f.route <;> simp

theorem on_observed_spec (d : ids.DestId) (live : recover.Live) :
    rollback.on_observed s d live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f, s.unit = some f ∧ f.dest = d ∧
        ((∃ i, WriteOk f live i ∧
            p = ({ s with phase := .Writing, install := some i }, .Mutate s.index d live i)) ∨
         (∃ st, st ≠ .Restored ∧ p = ({ s with phase := .Reporting }, .Report s.index d st))) ⦄ := by
  unfold rollback.on_observed
  cases he : s.unit with
  | none => simp [rollback.refuse, he]
  | some f =>
    simp only
    step with DestId.same_spec as ⟨ b, hb ⟩
    split_ifs with hd
    · have hfd : f.dest = d := hb.mp hd
      step with decide_restore_spec as ⟨ r, hw, hr ⟩
      cases r with
      | Write i =>
        simp only [WP.spec_ok]
        exact Or.inr ⟨f, rfl, hfd, Or.inl ⟨i, hw i rfl, by simp⟩⟩
      | Leave st =>
        simp only [rollback.report, WP.spec_ok]
        exact Or.inr ⟨f, rfl, hfd, Or.inr ⟨st, fun h => hr (by rw [h]), by simp [he]⟩⟩
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

end helpers

/-! ## The invariant of every run -/

/-- Phases after the plan was admitted. -/
def Planned (p : rollback.Phase) : Prop :=
  p = .ReadingUnit ∨ p = .Observing ∨ p = .Writing ∨ p = .Delegating ∨ p = .Reporting ∨
  p = .Flipping ∨ p = .Finished

/-- Unit `k` was reported settled in `cs`. -/
def Reported (k : U32) (cs : List rollback.Command) : Prop :=
  ∃ d st, rollback.Command.Report k d st ∈ cs

theorem Reported.mono {k : U32} {cs : List rollback.Command} (h : Reported k cs)
    (c : rollback.Command) : Reported k (cs ++ [c]) := by
  obtain ⟨d, st, h⟩ := h
  exact ⟨d, st, List.mem_append_left _ h⟩

structure Inv (s : rollback.RollbackState) (cs : List rollback.Command) : Prop where
  /-- Every unit before the one under decision was reported. -/
  reported : ∀ k : U32, k.val < s.index.val → Reported k cs
  /-- While reporting, the unit under decision was just reported. -/
  reporting : s.phase = .Reporting → Reported s.index cs
  /-- After the plan was read, it was admitted. -/
  admitted : Planned s.phase → Admitted s.plan
  /-- While writing, the outstanding write installs the unit's target. -/
  writing : s.phase = .Writing → ∃ f i, s.unit = some f ∧ s.install = some i ∧ Installs f.target i

/-- The executions of the machine: from `RollbackState::new`, any sequence of
events the step accepts, with the commands it issued, in order. -/
inductive Run : rollback.RollbackState → List rollback.Command → Prop where
  | start {s} : rollback.RollbackState.new = ok s → Run s []
  | step {s cs e s' c} : Run s cs → rollback.rollback_step s e = ok (s', c) → Run s' (cs ++ [c])

/-- A command that is neither a write, a restored report, nor the flip. -/
def Quiet (c : rollback.Command) : Prop :=
  (∀ u d x i, c ≠ .Mutate u d x i) ∧ (∀ u d, c ≠ .Report u d .Restored) ∧ (∀ sel, c ≠ .Flip sel)

/-- What one step guarantees, given the invariant before it. -/
def StepPost (s : rollback.RollbackState) (e : rollback.Event) (cs : List rollback.Command)
    (p : rollback.RollbackState × rollback.Command) : Prop :=
  Inv p.1 (cs ++ [p.2]) ∧
  (∀ u d x i, p.2 = .Mutate u d x i →
    s.phase = .Observing ∧ ∃ f, s.unit = some f ∧ e = .Observed d x ∧ f.dest = d ∧
      u = s.index ∧ WriteOk f x i) ∧
  (∀ u d, p.2 = .Report u d .Restored →
    s.phase = .Writing ∧ ∃ f i l, s.unit = some f ∧ s.install = some i ∧ Installs f.target i ∧
      e = .Landed l ∧ Lands i l) ∧
  (∀ sel, p.2 = .Flip sel →
    sel = p.1.plan.reselect ∧ Admitted p.1.plan ∧
      ∀ k : U32, k.val < p.1.plan.units.val → Reported k cs)

theorem StepPost.of_quiet {s e cs p} (hi : Inv p.1 (cs ++ [p.2])) (hq : Quiet p.2) :
    StepPost s e cs p :=
  ⟨hi, fun u d x i h => absurd h (hq.1 u d x i), fun u d h => absurd h (hq.2.1 u d),
    fun sel h => absurd h (hq.2.2 sel)⟩

/-- A step that keeps the index and plan, and moves to a phase that is not
reporting or writing, keeps the invariant. -/
theorem Inv.quiet {s : rollback.RollbackState} {cs : List rollback.Command} (h : Inv s cs)
    (s' : rollback.RollbackState) (c : rollback.Command)
    (hi : s'.index = s.index) (hp : s'.plan = s.plan)
    (h1 : s'.phase ≠ .Reporting) (h2 : s'.phase ≠ .Writing)
    (h3 : Planned s'.phase → Planned s.phase) : Inv s' (cs ++ [c]) where
  reported := fun k hk => (h.reported k (hi ▸ hk)).mono c
  reporting := fun hr => absurd hr h1
  admitted := fun hpl => hp ▸ h.admitted (h3 hpl)
  writing := fun hw => absurd hw h2

theorem post_abort {s : rollback.RollbackState} {e cs} (h : Inv s cs)
    (s' : rollback.RollbackState) (r : rollback.Refusal)
    (hi : s'.index = s.index) (hp : s'.plan = s.plan) (ha : s'.phase = .Aborted) :
    StepPost s e cs (s', .Abort r) :=
  StepPost.of_quiet (h.quiet _ _ hi hp (by simp [ha]) (by simp [ha])
    (fun hpl => by simp [Planned, ha] at hpl)) (by simp [Quiet])

/-- Entering `proceed` at index `t.index` with every earlier unit reported
and an admitted plan. -/
theorem post_proceed {s : rollback.RollbackState} {e cs} {t : rollback.RollbackState}
    {p : rollback.RollbackState × rollback.Command} (hp : Proceed t p)
    (hrep : ∀ k : U32, k.val < t.index.val → Reported k cs) (hadm : Admitted t.plan) :
    StepPost s e cs p := by
  rcases hp with ⟨hlt, rfl⟩ | ⟨hle, rfl⟩
  · refine StepPost.of_quiet ⟨fun k hk => (hrep k hk).mono _, by simp, fun _ => hadm, by simp⟩
      (by simp [Quiet])
  · refine ⟨⟨fun k hk => (hrep k hk).mono _, by simp, fun _ => hadm, by simp⟩, by simp, by simp, ?_⟩
    intro sel hsel
    simp only [rollback.Command.Flip.injEq] at hsel
    exact ⟨hsel.symm, hadm, fun k hk => hrep k (by simp only at hk ⊢; omega)⟩

theorem step_spec (s : rollback.RollbackState) (e : rollback.Event) (cs : List rollback.Command)
    (h : Inv s cs) : rollback.rollback_step s e ⦃ p => StepPost s e cs p ⦄ := by
  rcases s with ⟨phase, plan, index, unit, install⟩
  unfold rollback.rollback_step
  cases phase <;> cases e
  all_goals try first
    | (with_reducible exact WP.spec_mono (failed_or_protocol_spec' _ _) (by rintro p ⟨r, rfl⟩; exact post_abort h _ r rfl rfl rfl))
    | (with_reducible exact WP.spec_mono (refuse_spec' _ _) (by rintro p rfl; exact post_abort h _ _ rfl rfl rfl))
  case Start.Start =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Planned]))
      (by simp [Quiet])
  case ReadingPlan.PlanRead pl =>
    refine WP.spec_mono (on_plan_spec _ pl) ?_
    rintro p (⟨r, rfl⟩ | ⟨hadm, hp⟩)
    · exact post_abort h _ r rfl rfl rfl
    · exact post_proceed hp (fun k hk => by simp at hk) hadm
  case ReadingUnit.UnitRead f =>
    refine WP.spec_mono (on_unit_spec _ f) ?_
    rintro p (rfl | rfl) <;>
      exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (fun _ => by simp [Planned]))
        (by simp [Quiet])
  case Observing.Observed d live =>
    refine WP.spec_mono (on_observed_spec _ d live) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, hf, hfd, ⟨i, hw, rfl⟩ | ⟨st, hst, rfl⟩⟩)
    · exact post_abort h _ r rfl rfl rfl
    · refine ⟨⟨fun k hk => (h.reported k hk).mono _, by simp,
        fun _ => h.admitted (by simp [Planned]), fun _ => ⟨f, i, by simpa using hf, rfl, hw.1⟩⟩,
        ?_, by simp, by simp⟩
      intro u d' x i' hm
      simp only [rollback.Command.Mutate.injEq] at hm
      obtain ⟨rfl, rfl, rfl, rfl⟩ := hm
      exact ⟨rfl, f, hf, rfl, hfd, rfl, hw⟩
    · refine ⟨⟨fun k hk => (h.reported k hk).mono _, fun _ => ⟨d, st, by simp⟩,
        fun _ => h.admitted (by simp [Planned]), by simp⟩, by simp, ?_, by simp⟩
      intro u d' hr
      simp only [rollback.Command.Report.injEq] at hr
      exact absurd hr.2.2 hst
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
      refine ⟨⟨fun k hk => (h.reported k hk).mono _, fun _ => ⟨f.dest, .Restored, by simp⟩,
        fun _ => h.admitted (by simp [Planned]), by simp⟩, by simp, ?_, by simp⟩
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
      simp only at hk hi
      by_cases hlt : k.val < index.val
      · exact h.reported k hlt
      · have : k = index := UScalar.eq_of_val_eq (by omega)
        subst this
        exact h.reporting rfl
  case Flipping.Done =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (fun _ => by simp [Planned]))
      (by simp [Quiet])
  all_goals (
    refine WP.spec_mono (on_delegated_spec _ _) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, hf, -, rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    · exact ⟨⟨fun k hk => (h.reported k hk).mono _, fun _ => ⟨f.dest, .Delegated, by simp⟩,
        fun _ => h.admitted (by simp [Planned]), by simp⟩, by simp, by simp, by simp⟩)

/-! ## The theorems -/

/-- Every execution keeps the invariant. -/
theorem run_inv {s : rollback.RollbackState} {cs : List rollback.Command} (h : Run s cs) :
    Inv s cs := by
  induction h with
  | start hs =>
    simp only [rollback.RollbackState.new, generation.GenerationId.new, Std.bind_ok,
      ok.injEq] at hs
    subst hs
    exact ⟨fun k hk => by simp at hk, by simp, fun h => by simp [Planned] at h, by simp⟩
  | step _ hstep ih => exact (spec_eq_ok (step_spec _ _ _ ih) hstep).1

theorem run_post {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {c : rollback.Command} (hrun : Run s cs)
    (h : rollback.rollback_step s e = ok (s', c)) : StepPost s e cs (s', c) :=
  spec_eq_ok (step_spec _ _ _ (run_inv hrun)) h

/-- (K2) Rollback fidelity. In every execution, a write is issued only in
answer to the observation of the unit under decision, names that unit and its
destination, carries the observed object as `expect` (the driver re-checks it
before acting), and installs exactly the target generation's recorded
identity for that unit, from retained bytes — or removes gripsack's write
where the target deploys nothing. Nothing else is ever written. -/
theorem rollback_installs_target {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Mutate u d x i)) :
    s.phase = .Observing ∧ ∃ f, s.unit = some f ∧ e = .Observed d x ∧ f.dest = d ∧
      u = s.index ∧ Installs f.target i := by
  obtain ⟨hph, f, hf, he, hd, hu, hw⟩ := (run_post hrun h).2.1 _ _ _ _ rfl
  exact ⟨hph, f, hf, he, hd, hu, hw.1⟩

/-- (K1 through rollback) In every execution, a write replaces only absence
or gripsack's own write in the current generation: drift is never
overwritten, and rollback never assumes consent. -/
theorem rollback_write_k1 {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Mutate u d x i)) :
    ∃ f, s.unit = some f ∧ (x = .Absent ∨ Ours f.current x) := by
  obtain ⟨-, f, hf, -, -, -, hw⟩ := (run_post hrun h).2.1 _ _ _ _ rfl
  exact ⟨f, hf, hw.2.2⟩

/-- In every execution, no write targets a destination inside a published
generation's directory (FND-2's backfill was such a write). The flip, the
only other effect, names a selection. -/
theorem rollback_never_writes_generation {s s' : rollback.RollbackState}
    {cs : List rollback.Command} {e : rollback.Event} {u : U32} {d : ids.DestId}
    {x : recover.Live} {i : recover.Intended}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Mutate u d x i)) :
    ∃ f, s.unit = some f ∧ f.dest = d ∧ f.place = .Outside := by
  obtain ⟨-, f, hf, -, hd, -, hw⟩ := (run_post hrun h).2.1 _ _ _ _ rfl
  exact ⟨f, hf, hd, hw.2.1⟩

/-- In every execution, a unit is reported restored only when the object
that landed is exactly what the write installed — the target's recorded
identity (or the removal). -/
theorem rollback_restored_landed {s s' : rollback.RollbackState} {cs : List rollback.Command}
    {e : rollback.Event} {u : U32} {d : ids.DestId}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Report u d .Restored)) :
    s.phase = .Writing ∧ ∃ f i l, s.unit = some f ∧ s.install = some i ∧ Installs f.target i ∧
      e = .Landed l ∧ Lands i l :=
  (run_post hrun h).2.2.1 _ _ rfl

/-- In every execution, the flip is issued only after every unit of the plan
was reported settled — restored, already there, kept, untouched or settled
by the host's own domain — and names the planned selection. An unrestorable
unit aborts the run, so it never reaches the flip. -/
theorem rollback_flip_after_every_unit {s s' : rollback.RollbackState}
    {cs : List rollback.Command} {e : rollback.Event} {sel : selection.SelectionIdentity}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Flip sel)) :
    sel = s'.plan.reselect ∧ ∀ k : U32, k.val < s'.plan.units.val → Reported k cs := by
  obtain ⟨hs, -, hk⟩ := (run_post hrun h).2.2.2 _ rfl
  exact ⟨hs, hk⟩

/-- (K6) In every execution, the flip names a transaction selection whose
transaction differs from the one it replaces, and the target retains its own
shell profile: rollback reselects, it never writes into a generation. -/
theorem rollback_flip_admitted {s s' : rollback.RollbackState}
    {cs : List rollback.Command} {e : rollback.Event} {sel : selection.SelectionIdentity}
    (hrun : Run s cs) (h : rollback.rollback_step s e = ok (s', .Flip sel)) :
    s'.plan.profile = .Retained ∧ Fresh sel s'.plan.previous := by
  obtain ⟨hs, ⟨hp, hf⟩, -⟩ := (run_post hrun h).2.2.2 _ rfl
  exact ⟨hp, hs ▸ hf⟩

/-- R3: a unit whose target record no retained bytes reproduce, observed
absent, aborts the run naming the unit — it is never reported settled, so
the flip never follows. -/
theorem rollback_unrestorable_aborts (s : rollback.RollbackState) (f : rollback.UnitFacts)
    (o : recover.Ident) (hph : s.phase = .Observing) (hu : s.unit = some f)
    (ht : f.target = .Wrote o .Missing) :
    rollback.rollback_step s (.Observed f.dest .Absent) ⦃ p =>
      p.2 = .Abort (.Unrestorable s.index f.dest) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, plan, index, unit, install⟩
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
  rcases s with ⟨phase, plan, index, unit, install⟩
  simp only at hph hu
  subst hph hu
  simp [rollback.rollback_step, rollback.on_delegated, rollback.refuse]

end Gripsack.Kernel.Rollback
