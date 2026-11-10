import Extracted.GripsackKernel
import Kernel.Selection
import Kernel.Recover
import Kernel.Ownership
import Kernel.Rollback
import Kernel.Publication
import Kernel.Retention

/-!
# The apply machine (plan/0056 F2 step 7)

Theorems over the Aeneas translation of `gripsack_kernel::apply`
(`Extracted/GripsackKernel.lean`).

* `decide_apply_spec`: the per-unit decision writes only what the unit
  wants — the deploy's identity, the adoption's under consent, or the
  release's — with an authority the observation justifies (absence,
  gripsack's own last write, or consent for a deploy: K1); it never claims
  the unit applied.
* `decide_apply_drift_never_written` (K1): without consent, a live object
  that is not gripsack's last write is never written.
* `decide_apply_refused`: a deploy that may not touch the user's object and
  must not keep it silently refuses the run.
* (K2) `apply_installs_intended`: over every run, a write answers the
  observation of the unit under decision, carries that observation as
  `expect`, and installs exactly what the unit wants.
* `apply_write_k1`: over every run, a write replaces only absence,
  gripsack's own last write, or — under consent, for a deploy — the live
  object.
* `apply_applied_landed`: over every run, a unit is reported applied only
  when the landed object is exactly what the write installed; kept, already
  satisfied, declared and delegated units never claim it.
* (FND-3) `apply_release_undeclared`: over every run, a release is written
  only for a destination no deploy unit of the run claims, and only once
  releases began — `apply_no_deploy_after_release`: no deploy unit is
  accepted after that.
* `apply_flip_after_every_unit`: over every run, the flip comes only after
  every unit read was reported settled (a refused unit aborts, so it never
  reaches the flip) and after the publication.
* (K6) `apply_flip_admitted`: over every run, the flip names the reserved
  selection of an admitted publication: a fresh transaction bound to the
  allocated generation, above the floor, not yet published, never for a
  vacuous run — and the run's `touched` fact is the machine's own record of
  whether it changed the disk.
* `apply_vacuous_untouched`: over every run, a run that reported any unit
  applied never finishes vacuous.
* `apply_refused_aborts`, `apply_aborted_terminal`: a refusal ends the run,
  and nothing — in particular no flip — follows an abort.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.Apply

/-! ## The per-unit decision -/

/-- What a write with this authority installs for the unit's want. -/
def Installs : apply.Want → apply.Write → recover.Intended → Prop
  | .Deploy o _, .Create, i => i = .Obj o
  | .Deploy o _, .Replace, i => i = .Obj o
  | .Deploy _ a, .TakeOver, i => i = .Obj a
  | .Release r, .Replace, i => i = r
  | .Release _, .Create, _ => False
  | .Release _, .TakeOver, _ => False

/-- K1: the authority a write carries is what the observation allows —
absence, gripsack's own last write, or consent to absorb the live object for
a deploy. -/
def Authorized (f : apply.UnitFacts) (live : recover.Live) : apply.Write → Prop
  | .Create => live = .Absent
  | .Replace => Ours f.recorded live
  | .TakeOver => f.consent.take_over = .Granted ∧ (∃ o, live = .Obj o) ∧ ∃ o a, f.want = .Deploy o a

/-- A write the apply may issue for unit `f` over the observed `live`. -/
def WriteOk (f : apply.UnitFacts) (live : recover.Live) (i : recover.Intended)
    (w : apply.Write) : Prop :=
  Installs f.want w i ∧ Authorized f live w

/-- The unfolding the decision claims start from. -/
macro "apply_cases" : tactic => `(tactic| (
  simp only [apply.decide_apply, apply.asked, apply.written, apply.install, apply.refusal_stops,
    apply.stops, ownership.decide_unit, ownership.Recorded.wrote, ownership.acquires,
    ownership.Consent.grants_take_over, ownership.refusal, recover.intends, Ident.same_ok,
    decide_eq_true_eq]
  all_goals (try simp only [bind_tc_ok, bind_ok, decide_eq_true_eq])
  all_goals (try split_ifs)
  all_goals (repeat' split)
  all_goals simp_all [WriteOk, Installs, Authorized, Ours, WP.spec_ok]))

set_option maxHeartbeats 1600000 in
/-- The decision writes only what `WriteOk` allows, and never claims the
unit applied: only a landed write is reported applied. -/
theorem decide_apply_spec (k : U32) (f : apply.UnitFacts) (live : recover.Live) :
    apply.decide_apply k f live ⦃ r =>
      (∀ i w, r = .Write i w → WriteOk f live i w) ∧ r ≠ .Leave .Applied ⦄ := by
  obtain ⟨dest, slot, route, recorded, want, ⟨t⟩, drift⟩ := f
  cases want with
  | Deploy o a => cases live <;> cases recorded <;> cases t <;> cases drift <;> apply_cases
  | Release r => cases r <;> cases live <;> cases recorded <;> cases t <;> cases drift <;> apply_cases

/-- K1 through apply: without consent, a live object that is not gripsack's
last write is never written — preserved drift, a user's replacement and an
unrecorded object alike. -/
theorem decide_apply_drift_never_written (k : U32) (f : apply.UnitFacts) (o : recover.Ident)
    (hc : f.consent.take_over = .Withheld) (hr : f.recorded ≠ .Wrote o) :
    apply.decide_apply k f (.Obj o) ⦃ r => ∀ i w, r ≠ .Write i w ⦄ := by
  refine WP.spec_mono (decide_apply_spec k f (.Obj o)) ?_
  rintro r ⟨hw, -⟩ i w rfl
  obtain ⟨-, ha⟩ := hw i w rfl
  cases w with
  | Create => simp [Authorized] at ha
  | Replace =>
    obtain ⟨o', ho, hro⟩ := ha
    cases ho
    exact hr hro
  | TakeOver => simp [Authorized, hc] at ha

/-- A deploy whose unit says to stop on the user's object refuses the run,
naming the unit, when the live object is not gripsack's last write, no
consent was given and the object is not already the intent. -/
theorem decide_apply_refused (k : U32) (f : apply.UnitFacts) (o i a : recover.Ident)
    (hd : f.drift = .Stop) (hw : f.want = .Deploy i a) (hio : i ≠ o)
    (hc : f.consent.take_over = .Withheld) (hr : f.recorded ≠ .Wrote o) :
    apply.decide_apply k f (.Obj o) ⦃ r => ∃ why, r = .Refuse (.Refused k f.dest why) ⦄ := by
  obtain ⟨dest, slot, route, recorded, want, ⟨t⟩, drift⟩ := f
  simp only at hd hw hc hr
  subst hd hw hc
  cases recorded <;> apply_cases

/-! ## Release coverage (FND-3) -/

/-- A deploy's claim covers a release at `d`: any deploy at the destination
covers a whole-object release; only the same block covers a block's. -/
def Covers (c : apply.Claim) (d : ids.DestId) : apply.Slot → Prop
  | .Whole => c.dest = d
  | .Block o => c.dest = d ∧ c.slot = .Block o

/-- Some claim in the list covers the release. -/
def Covered (cl : List apply.Claim) (d : ids.DestId) (sl : apply.Slot) : Prop :=
  ∃ c ∈ cl, Covers c d sl

def IsRelease : apply.Want → Prop
  | .Release _ => True
  | .Deploy _ _ => False

theorem covers_spec (c : apply.Claim) (d : ids.DestId) (sl : apply.Slot) :
    apply.covers c d sl ⦃ b => b ↔ Covers c d sl ⦄ := by
  obtain ⟨cd, cs⟩ := c
  unfold apply.covers
  step with DestId.same_spec as ⟨ b, hb ⟩
  split_ifs with h
  · have hcd : cd = d := hb.mp h
    cases sl with
    | Whole => simp [Covers, hcd]
    | Block o => cases cs <;> simp [Covers, hcd, eq_comm]
  · have hcd : cd ≠ d := fun e => h (hb.mpr e)
    cases sl <;> simp [Covers, hcd]

theorem covered_loop_spec (cl : Slice apply.Claim) (d : ids.DestId) (sl : apply.Slot) (i : Usize)
    (hi : i.val ≤ cl.val.length)
    (hprefix : ∀ j (h : j < cl.val.length), j < i.val → ¬ Covers cl.val[j] d sl) :
    apply.covered_loop cl d sl i ⦃ b => b ↔ Covered cl.val d sl ⦄ := by
  unfold apply.covered_loop
  apply loop.spec_decr_nat (fun i : Usize => cl.val.length - i.val)
    (fun i : Usize => i.val ≤ cl.val.length ∧
      ∀ j (h : j < cl.val.length), j < i.val → ¬ Covers cl.val[j] d sl)
  · intro i ⟨hi, hprefix⟩
    unfold apply.covered_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < cl.val.length := by scalar_tac
      step as ⟨ c, hc ⟩
      step with covers_spec as ⟨ b, hb ⟩
      split_ifs with hcov
      · simp only [WP.spec_ok]
        exact ⟨fun _ => ⟨c, by rw [hc]; exact List.getElem_mem hlt', hb.mp hcov⟩, fun _ => by simp⟩
      · apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i2 hi2
        simp only [WP.spec_ok]
        and_intros
        all_goals try scalar_tac
        intro j hj hji
        by_cases hlt'' : j < i.val
        · exact hprefix j hj hlt''
        · have : j = i.val := by scalar_tac
          subst this
          rw [← hc]
          exact fun h => hcov (hb.mpr h)
    · simp only [WP.spec_ok]
      refine ⟨fun h => (by cases h), ?_⟩
      rintro ⟨c, hc, hcov⟩
      obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hc
      exact absurd hcov (hprefix j hj (by scalar_tac))
  · exact ⟨hi, hprefix⟩

theorem covered_spec (cl : Slice apply.Claim) (d : ids.DestId) (sl : apply.Slot) :
    apply.covered cl d sl ⦃ b => b ↔ Covered cl.val d sl ⦄ :=
  covered_loop_spec cl d sl 0#usize (by simp) (fun j _ hj => by simp at hj)

/-! ## What each step helper returns -/

/-- The commit facts the machine builds: the host's observation, and its own
record of whether the run changed the disk. -/
def factsOf (inp : apply.CommitInputs) (m : Bool) : publication.CommitFacts :=
  { previous := inp.previous, reserved := inp.reserved, allocated := inp.allocated,
    floor := inp.floor, unchanged := inp.unchanged, touched := m, «exists» := inp.exists }

theorem commit_facts_ok (inp : apply.CommitInputs) (m : Bool) :
    apply.commit_facts inp m = ok (factsOf inp m) := by
  simp [apply.commit_facts, factsOf]

section helpers
variable (s : apply.ApplyState)

theorem refuse_spec' (r : apply.Refusal) :
    apply.refuse s r ⦃ p => p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  simp [apply.refuse]

theorem failed_or_protocol_spec' (e : apply.Event) :
    apply.failed_or_protocol s e ⦃ p => ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold apply.failed_or_protocol
  cases e <;> simp [apply.refuse]

/-- Where `advance` takes the run: the unit after `s.index`, or a refusal
when the counter would overflow. -/
def Advance (p : apply.ApplyState × apply.Command) : Prop :=
  p = ({ s with unit := none, install := none, phase := .Aborted }, .Abort .Malformed) ∨
  ∃ i : U32, i.val = s.index.val + 1 ∧
    p = ({ s with index := i, unit := none, install := none, phase := .ReadingUnit }, .ReadUnit i)

theorem advance_spec : apply.advance s ⦃ p => Advance s p ⦄ := by
  unfold apply.advance Advance
  have h := U32.checked_add_bv_spec s.index 1#u32
  cases ho : U32.checked_add s.index 1#u32 with
  | none => simp [lift, apply.refuse]
  | some i =>
    simp only [ho] at h
    simp only [lift, Std.bind_ok, apply.read_unit, WP.spec_ok]
    right
    exact ⟨i, by simpa using h.2.1, rfl⟩

theorem examine_spec (f : apply.UnitFacts) :
    apply.examine s f ⦃ p =>
      p = ({ s with phase := .Observing, unit := some f }, .Observe s.index f.dest) ∨
      p = ({ s with phase := .Delegating, unit := some f }, .Delegate s.index f.dest) ⦄ := by
  unfold apply.examine
  cases f.route <;> simp

/-- Where a unit read takes the run. -/
def OnUnit (f : apply.UnitFacts) (p : apply.ApplyState × apply.Command) : Prop :=
  (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
  (¬ IsRelease f.want ∧ ∃ v : alloc.vec.Vec apply.Claim,
    v.val = s.declared.val ++ [⟨f.dest, f.slot⟩] ∧
    (p = ({ s with declared := v, phase := .Observing, unit := some f }, .Observe s.index f.dest) ∨
     p = ({ s with declared := v, phase := .Delegating, unit := some f }, .Delegate s.index f.dest))) ∨
  (IsRelease f.want ∧ Covered s.declared.val f.dest f.slot ∧
    p = ({ s with unit := some f, releasing := true, phase := .Reporting },
      .Report s.index f.dest .Declared)) ∨
  (IsRelease f.want ∧ ¬ Covered s.declared.val f.dest f.slot ∧
    (p = ({ s with releasing := true, phase := .Observing, unit := some f },
        .Observe s.index f.dest) ∨
     p = ({ s with releasing := true, phase := .Delegating, unit := some f },
        .Delegate s.index f.dest)))

theorem on_unit_spec (f : apply.UnitFacts) : apply.on_unit s f ⦃ p => OnUnit s f p ⦄ := by
  unfold apply.on_unit OnUnit
  cases hw : f.want with
  | Deploy o a =>
    simp only
    split_ifs with hr hmax
    · simp [apply.refuse]
    · simp [apply.refuse]
    · have hpush : s.declared.val.length < Usize.max := by
        have := alloc.vec.Vec.len_val s.declared
        have hne : (alloc.vec.Vec.len s.declared).val ≠ Usize.max := by
          intro e
          apply hmax
          exact UScalar.eq_of_val_eq (by simpa [core.num.Usize.MAX] using e)
        have := s.declared.property
        simp only [alloc.vec.Vec.len_val] at hne
        scalar_tac
      apply WP.spec_bind (alloc.vec.Vec.push_spec _ _ hpush)
      intro v hv
      refine WP.spec_mono (examine_spec _ f) ?_
      intro p hp
      right; left
      refine ⟨by simp [IsRelease], v, hv, ?_⟩
      simpa using hp
  | Release r =>
    simp only
    step with covered_spec as ⟨ b, hb ⟩
    simp only [deref_val] at hb
    split_ifs with hcov
    · simp only [apply.report, WP.spec_ok]
      right; right; left
      exact ⟨by simp [IsRelease], hb.mp hcov, by simp⟩
    · refine WP.spec_mono (examine_spec _ f) ?_
      intro p hp
      right; right; right
      exact ⟨by simp [IsRelease], fun h => hcov (hb.mpr h), by simpa using hp⟩

theorem on_observed_spec (d : ids.DestId) (live : recover.Live) :
    apply.on_observed s d live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f, s.unit = some f ∧ f.dest = d ∧
        ((∃ i w, WriteOk f live i w ∧
            p = ({ s with phase := .Writing, install := some i }, .Mutate s.index d live i w)) ∨
         (∃ st, st ≠ .Applied ∧ p = ({ s with phase := .Reporting }, .Report s.index d st))) ⦄ := by
  unfold apply.on_observed
  cases he : s.unit with
  | none => simp [apply.refuse, he]
  | some f =>
    simp only
    step with DestId.same_spec as ⟨ b, hb ⟩
    split_ifs with hd
    · have hfd : f.dest = d := hb.mp hd
      step with decide_apply_spec as ⟨ r, hw, hr ⟩
      cases r with
      | Write i w =>
        simp only [WP.spec_ok]
        exact Or.inr ⟨f, rfl, hfd, Or.inl ⟨i, w, hw i w rfl, by simp⟩⟩
      | Leave st =>
        simp only [apply.report, WP.spec_ok]
        exact Or.inr ⟨f, rfl, hfd, Or.inr ⟨st, fun h => hr (by rw [h]), by simp [he]⟩⟩
      | Refuse r => simp [apply.refuse, he]
    · simp [apply.refuse, he]

theorem on_landed_spec (live : recover.Live) :
    apply.on_landed s live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f i, s.unit = some f ∧ s.install = some i ∧ Rollback.Lands i live ∧
        p = ({ s with mutated := true, phase := .Reporting }, .Report s.index f.dest .Applied) ⦄ := by
  unfold apply.on_landed
  cases he : s.unit with
  | none => simp [apply.refuse, he]
  | some f =>
    cases hi : s.install with
    | none => simp [apply.refuse, he, hi]
    | some i =>
      simp only
      step with Rollback.lands_spec as ⟨ b, hb ⟩
      split_ifs with h
      · simp only [apply.report, WP.spec_ok]
        exact Or.inr ⟨f, i, rfl, rfl, hb.mp h, by simp⟩
      · simp [apply.refuse, he, hi]

theorem on_write_failed_spec (failure : recover.Failure) :
    apply.on_write_failed s failure ⦃ p =>
      ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold apply.on_write_failed
  cases failure <;> cases he : s.unit <;> simp [apply.refuse, he]

theorem on_delegated_spec (e : apply.Event) :
    apply.on_delegated s e ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f m, s.unit = some f ∧ e = .Delegated m ∧
        p = ({ s with mutated := (m || s.mutated), phase := .Reporting },
          .Report s.index f.dest (.Delegated m)) ⦄ := by
  unfold apply.on_delegated
  cases he : s.unit with
  | none => simp [apply.refuse, he]
  | some f =>
    cases e with
    | Delegated m => cases m <;> simp [apply.report]
    | _ => simp [apply.failed_or_protocol, apply.refuse, he]

theorem on_commit_spec (inp : apply.CommitInputs) :
    apply.on_commit s inp ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      (Publication.Vacuous (factsOf inp s.mutated) ∧
        p = ({ s with phase := .Finished }, .Finish .Vacuous)) ∨
      (Publication.Admitted (factsOf inp s.mutated) ∧ ¬ Publication.Vacuous (factsOf inp s.mutated) ∧
        p = ({ s with phase := .Publishing, commit := some (factsOf inp s.mutated) },
          .Publish (factsOf inp s.mutated).allocated)) ⦄ := by
  unfold apply.on_commit
  rw [commit_facts_ok, Std.bind_ok]
  obtain ⟨c, hc, hpub⟩ := spec_exists_ok (Publication.publish_admitted (factsOf inp s.mutated))
  obtain ⟨c', hc', hvac⟩ := spec_exists_ok (Publication.vacuous_iff (factsOf inp s.mutated))
  rw [hc] at hc'
  simp only [ok.injEq] at hc'
  subst hc'
  rw [hc, Std.bind_ok]
  cases c with
  | Vacuous => simp only [WP.spec_ok]; exact Or.inr (Or.inl ⟨hvac.mp rfl, by simp⟩)
  | Publish =>
    obtain ⟨ha, hv⟩ := hpub rfl
    simp only [WP.spec_ok]
    exact Or.inr (Or.inr ⟨ha, hv, by simp⟩)
  | Refuse r => simp [apply.refuse]

theorem on_published_spec :
    apply.on_published s ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f, s.commit = some f ∧ p = ({ s with phase := .Flipping }, .Flip f.reserved) ⦄ := by
  unfold apply.on_published
  cases he : s.commit <;> simp [apply.refuse, he]

end helpers

/-! ## The invariant of every run -/

/-- Unit `k` was reported settled in `cs`. -/
def Reported (k : U32) (cs : List apply.Command) : Prop :=
  ∃ d st, apply.Command.Report k d st ∈ cs

theorem Reported.mono {k : U32} {cs : List apply.Command} (h : Reported k cs)
    (c : apply.Command) : Reported k (cs ++ [c]) := by
  obtain ⟨d, st, h⟩ := h
  exact ⟨d, st, List.mem_append_left _ h⟩

/-- Some unit was reported applied in `cs`. -/
def Applied (cs : List apply.Command) : Prop :=
  ∃ k d, apply.Command.Report k d .Applied ∈ cs

/-- A unit is under examination (observed, written or delegated). -/
def Examining (p : apply.Phase) : Prop :=
  p = .Observing ∨ p = .Writing ∨ p = .Delegating

/-- A publication was admitted and the flip is pending or done. -/
def Committing (p : apply.Phase) : Prop :=
  p = .Publishing ∨ p = .Flipping ∨ p = .Activating

/-- The admitted commit: the publication's facts, admitted, not vacuous, and
recording as `touched` exactly the machine's own `mutated` flag. -/
def CommitOk (s : apply.ApplyState) : Prop :=
  ∃ f, s.commit = some f ∧ Publication.Admitted f ∧ ¬ Publication.Vacuous f ∧ f.touched = s.mutated

structure Inv (s : apply.ApplyState) (cs : List apply.Command) : Prop where
  /-- Every unit before the one under decision was reported. -/
  reported : ∀ k : U32, k.val < s.index.val → Reported k cs
  /-- While reporting, the unit under decision was just reported. -/
  reporting : s.phase = .Reporting → Reported s.index cs
  /-- While writing, the outstanding write installs what the unit wants. -/
  writing : s.phase = .Writing →
    ∃ f i w, s.unit = some f ∧ s.install = some i ∧ Installs f.want w i
  /-- A release under examination is claimed by no deploy, and releases began. -/
  unclaimed : Examining s.phase → ∀ f, s.unit = some f → IsRelease f.want →
    ¬ Covered s.declared.val f.dest f.slot ∧ s.releasing = true
  /-- A reported application set the `mutated` flag. -/
  applied : Applied cs → s.mutated = true
  /-- From the publication on, the commit was admitted. -/
  committed : Committing s.phase → CommitOk s

/-- The executions of the machine: from `ApplyState::new`, any sequence of
events the step accepts, with the commands it issued, in order. -/
inductive Run : apply.ApplyState → List apply.Command → Prop where
  | start {s} : apply.ApplyState.new = ok s → Run s []
  | step {s cs e s' c} : Run s cs → apply.apply_step s e = ok (s', c) → Run s' (cs ++ [c])

/-- A command none of the claims is about. -/
def Quiet (c : apply.Command) : Prop :=
  (∀ u d x i w, c ≠ .Mutate u d x i w) ∧ (∀ u d, c ≠ .Report u d .Applied) ∧
    (∀ sel, c ≠ .Flip sel) ∧ c ≠ .Finish .Vacuous

/-- What one step guarantees, given the invariant before it. -/
def StepPost (s : apply.ApplyState) (e : apply.Event) (cs : List apply.Command)
    (p : apply.ApplyState × apply.Command) : Prop :=
  Inv p.1 (cs ++ [p.2]) ∧
  (∀ u d x i w, p.2 = .Mutate u d x i w →
    s.phase = .Observing ∧ ∃ f, s.unit = some f ∧ e = .Observed d x ∧ f.dest = d ∧
      u = s.index ∧ WriteOk f x i w ∧
      (IsRelease f.want → ¬ Covered s.declared.val d f.slot ∧ s.releasing = true)) ∧
  (∀ u d, p.2 = .Report u d .Applied →
    s.phase = .Writing ∧ ∃ f i w l, s.unit = some f ∧ s.install = some i ∧
      Installs f.want w i ∧ e = .Landed l ∧ Rollback.Lands i l) ∧
  (∀ sel, p.2 = .Flip sel →
    ∃ f, p.1.commit = some f ∧ sel = f.reserved ∧ Publication.Admitted f ∧
      ¬ Publication.Vacuous f ∧ f.touched = p.1.mutated ∧
      ∀ k : U32, k.val < p.1.index.val → Reported k cs) ∧
  (p.2 = .Finish .Vacuous → s.mutated = false)

theorem StepPost.of_quiet {s e cs p} (hi : Inv p.1 (cs ++ [p.2])) (hq : Quiet p.2) :
    StepPost s e cs p :=
  ⟨hi, fun u d x i w h => absurd h (hq.1 u d x i w), fun u d h => absurd h (hq.2.1 u d),
    fun sel h => absurd h (hq.2.2.1 sel), fun h => absurd h hq.2.2.2⟩

theorem applied_append {cs : List apply.Command} {c : apply.Command}
    (hc : ∀ k d, c ≠ .Report k d .Applied) (h : Applied (cs ++ [c])) : Applied cs := by
  obtain ⟨k, d, hm⟩ := h
  rcases List.mem_append.mp hm with hm | hm
  · exact ⟨k, d, hm⟩
  · simp only [List.mem_singleton] at hm
    exact absurd hm.symm (hc k d)

/-- A step that keeps the index, the flag and the commit, and moves to a
phase that is neither reporting, writing nor examining, keeps the invariant.
-/
theorem Inv.quiet {s : apply.ApplyState} {cs : List apply.Command} (h : Inv s cs)
    (s' : apply.ApplyState) (c : apply.Command)
    (hi : s'.index = s.index) (hm : s'.mutated = s.mutated)
    (h1 : s'.phase ≠ .Reporting) (h2 : s'.phase ≠ .Writing) (h3 : ¬ Examining s'.phase)
    (h4 : Committing s'.phase → Committing s.phase ∧ s'.commit = s.commit)
    (hc : ∀ k d, c ≠ .Report k d .Applied) : Inv s' (cs ++ [c]) where
  reported := fun k hk => (h.reported k (hi ▸ hk)).mono c
  reporting := fun hr => absurd hr h1
  writing := fun hw => absurd hw h2
  unclaimed := fun he => absurd he h3
  applied := fun ha => hm ▸ h.applied (applied_append hc ha)
  committed := fun hc' => by
    obtain ⟨hc0, hcm⟩ := h4 hc'
    obtain ⟨f, hf, ha, hv, ht⟩ := h.committed hc0
    exact ⟨f, hcm ▸ hf, ha, hv, hm ▸ ht⟩

theorem post_abort {s : apply.ApplyState} {e cs} (h : Inv s cs)
    (s' : apply.ApplyState) (r : apply.Refusal)
    (hi : s'.index = s.index) (hm : s'.mutated = s.mutated) (ha : s'.phase = .Aborted) :
    StepPost s e cs (s', .Abort r) :=
  StepPost.of_quiet (h.quiet _ _ hi hm (by simp [ha]) (by simp [ha]) (by simp [Examining, ha])
    (fun hc => by simp [Committing, ha] at hc) (by simp)) (by simp [Quiet])

theorem step_spec (s : apply.ApplyState) (e : apply.Event) (cs : List apply.Command)
    (h : Inv s cs) : apply.apply_step s e ⦃ p => StepPost s e cs p ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, releasing, mutated, commit⟩
  unfold apply.apply_step apply.on_reading
  cases phase <;> cases e
  all_goals try first
    | (with_reducible exact WP.spec_mono (failed_or_protocol_spec' _ _) (by rintro p ⟨r, rfl⟩; exact post_abort h _ r rfl rfl rfl))
    | (with_reducible exact WP.spec_mono (refuse_spec' _ _) (by rintro p rfl; exact post_abort h _ _ rfl rfl rfl))
  case Start.Start =>
    simp only [apply.read_unit, WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (by simp [Committing]) (by simp)) (by simp [Quiet])
  case ReadingUnit.UnitRead f =>
    refine WP.spec_mono (on_unit_spec _ f) ?_
    rintro p (⟨r, rfl⟩ | ⟨hd, v, hv, rfl | rfl⟩ | ⟨hrel, hcov, rfl⟩ | ⟨hrel, hcov, rfl | rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    all_goals first
      | (refine StepPost.of_quiet ⟨fun k hk => (h.reported k hk).mono _, by simp, by simp,
          ?_, fun ha => h.applied (applied_append (by simp) ha), by simp [Committing]⟩
          (by simp [Quiet])
         intro _ f' hf' hr'
         simp only [Option.some.injEq] at hf'
         subst hf'
         first | exact absurd hr' hd | exact ⟨hcov, rfl⟩)
      | (refine StepPost.of_quiet ⟨fun k hk => (h.reported k hk).mono _,
          fun _ => ⟨f.dest, .Declared, by simp⟩, by simp, by simp [Examining],
          fun ha => h.applied (applied_append (by simp) ha), by simp [Committing]⟩
          (by simp [Quiet]))
  case ReadingUnit.Exhausted =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (by simp [Committing]) (by simp)) (by simp [Quiet])
  case Observing.Observed d live =>
    refine WP.spec_mono (on_observed_spec _ d live) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, hf, hfd, ⟨i, w, hw, rfl⟩ | ⟨st, hst, rfl⟩⟩)
    · exact post_abort h _ r rfl rfl rfl
    · have hu := h.unclaimed (by simp [Examining]) f hf
      refine ⟨⟨fun k hk => (h.reported k hk).mono _, by simp,
        fun _ => ⟨f, i, w, by simpa using hf, rfl, hw.1⟩, ?_,
        fun ha => h.applied (applied_append (by simp) ha), by simp [Committing]⟩,
        ?_, by simp, by simp, by simp⟩
      · intro _ f' hf' hr'
        simp only at hf hf'
        rw [hf] at hf'
        cases hf'
        exact hu hr'
      · intro u d' x i' w' hm
        simp only [apply.Command.Mutate.injEq] at hm
        obtain ⟨rfl, rfl, rfl, rfl, rfl⟩ := hm
        exact ⟨rfl, f, hf, rfl, hfd, rfl, hw, fun hr => hfd ▸ hu hr⟩
    · refine ⟨⟨fun k hk => (h.reported k hk).mono _, fun _ => ⟨d, st, by simp⟩, by simp,
        by simp [Examining], fun ha => h.applied (applied_append ?_ ha), by simp [Committing]⟩,
        by simp, ?_, by simp, by simp⟩
      · intro k d' hc
        simp only [apply.Command.Report.injEq] at hc
        exact hst hc.2.2
      · intro u d' hr
        simp only [apply.Command.Report.injEq] at hr
        exact absurd hr.2.2 hst
  case Writing.Landed live =>
    refine WP.spec_mono (on_landed_spec _ live) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, i, hf, hi, hl, rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    · obtain ⟨f', i', w, hf', hi', hins⟩ := h.writing rfl
      simp only at hf hi hf' hi'
      rw [hf] at hf'
      rw [hi] at hi'
      cases hf'
      cases hi'
      refine ⟨⟨fun k hk => (h.reported k hk).mono _, fun _ => ⟨f.dest, .Applied, by simp⟩,
        by simp, by simp [Examining], fun _ => rfl, by simp [Committing]⟩, by simp, ?_, by simp,
        by simp⟩
      intro u d hr
      exact ⟨rfl, f, i, w, live, hf, hi, hins, rfl, hl⟩
  case Writing.Failed failure =>
    exact WP.spec_mono (on_write_failed_spec _ failure)
      (by rintro p ⟨r, rfl⟩; exact post_abort h _ r rfl rfl rfl)
  case Reporting.Done =>
    refine WP.spec_mono (advance_spec _) ?_
    rintro p (rfl | ⟨i, hi, rfl⟩)
    · exact post_abort h _ _ rfl rfl rfl
    · refine StepPost.of_quiet ⟨fun k hk => ?_, by simp, by simp, by simp [Examining],
        fun ha => h.applied (applied_append (by simp) ha), by simp [Committing]⟩ (by simp [Quiet])
      simp only at hk hi
      by_cases hlt : k.val < index.val
      · exact (h.reported k hlt).mono _
      · have : k = index := UScalar.eq_of_val_eq (by omega)
        subst this
        exact (h.reporting rfl).mono _
  case Directories.Done =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (by simp [Committing]) (by simp)) (by simp [Quiet])
  case ReadingCommit.CommitRead inp =>
    refine WP.spec_mono (on_commit_spec _ inp) ?_
    rintro p (⟨r, rfl⟩ | ⟨hv, rfl⟩ | ⟨ha, hv, rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    · refine ⟨h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
        (by simp [Committing]) (by simp), by simp, by simp, by simp, fun _ => ?_⟩
      exact by simpa [factsOf] using hv.2.2
    · exact StepPost.of_quiet ⟨fun k hk => (h.reported k hk).mono _, by simp, by simp,
        by simp [Examining], fun ha' => h.applied (applied_append (by simp) ha'),
        fun _ => ⟨_, rfl, ha, hv, rfl⟩⟩ (by simp [Quiet])
  case Publishing.Done =>
    refine WP.spec_mono (on_published_spec _) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, hf, rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    · obtain ⟨f', hf', ha, hv, ht⟩ := h.committed (by simp [Committing])
      simp only at hf hf'
      rw [hf] at hf'
      cases hf'
      refine ⟨h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
        (fun _ => ⟨by simp [Committing], rfl⟩) (by simp), by simp, by simp, ?_, by simp⟩
      intro sel hsel
      simp only [apply.Command.Flip.injEq] at hsel
      exact ⟨f, hf, hsel.symm, ha, hv, ht, fun k hk => h.reported k hk⟩
  case Flipping.Done =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (fun _ => ⟨by simp [Committing], rfl⟩) (by simp)) (by simp [Quiet])
  case Activating.Done =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (by simp [Committing]) (by simp)) (by simp [Quiet])
  all_goals (
    refine WP.spec_mono (on_delegated_spec _ _) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, m, hf, he, rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    · refine StepPost.of_quiet ⟨fun k hk => (h.reported k hk).mono _,
        fun _ => ⟨f.dest, .Delegated m, by simp⟩, by simp, by simp [Examining], ?_,
        by simp [Committing]⟩ (by simp [Quiet])
      intro ha
      have := h.applied (applied_append (by simp) ha)
      simp only at this
      simp [this])

/-! ## The theorems -/

/-- Every execution keeps the invariant. -/
theorem run_inv {s : apply.ApplyState} {cs : List apply.Command} (h : Run s cs) : Inv s cs := by
  induction h with
  | start hs =>
    simp only [apply.ApplyState.new, ok.injEq] at hs
    subst hs
    exact ⟨fun k hk => by simp at hk, by simp, by simp, by simp [Examining],
      fun ⟨k, d, hm⟩ => by simp at hm, by simp [Committing]⟩
  | step _ hstep ih => exact (spec_eq_ok (step_spec _ _ _ ih) hstep).1

theorem run_post {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {c : apply.Command} (hrun : Run s cs)
    (h : apply.apply_step s e = ok (s', c)) : StepPost s e cs (s', c) :=
  spec_eq_ok (step_spec _ _ _ (run_inv hrun)) h

/-- (K2) Apply fidelity. In every execution, a write is issued only in
answer to the observation of the unit under decision, names that unit and
its destination, carries the observed object as `expect` (the driver
re-checks it before acting), and installs exactly what the unit wants: the
deploy's identity, the adoption's under consent, or the release's. -/
theorem apply_installs_intended {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    {w : apply.Write}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Mutate u d x i w)) :
    s.phase = .Observing ∧ ∃ f, s.unit = some f ∧ e = .Observed d x ∧ f.dest = d ∧
      u = s.index ∧ Installs f.want w i := by
  obtain ⟨hph, f, hf, he, hd, hu, hw, -⟩ := (run_post hrun h).2.1 _ _ _ _ _ rfl
  exact ⟨hph, f, hf, he, hd, hu, hw.1⟩

/-- (K1 through apply) In every execution, a write replaces only absence,
gripsack's own last write, or — when the user consented, and only for a
deploy — the live object. -/
theorem apply_write_k1 {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    {w : apply.Write}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Mutate u d x i w)) :
    ∃ f, s.unit = some f ∧
      (x = .Absent ∨ Ours f.recorded x ∨
        (f.consent.take_over = .Granted ∧ (∃ o, x = .Obj o) ∧ ∃ o a, f.want = .Deploy o a)) := by
  obtain ⟨-, f, hf, -, -, -, ⟨-, ha⟩, -⟩ := (run_post hrun h).2.1 _ _ _ _ _ rfl
  refine ⟨f, hf, ?_⟩
  cases w with
  | Create => exact Or.inl ha
  | Replace => exact Or.inr (Or.inl ha)
  | TakeOver => exact Or.inr (Or.inr ha)

/-- In every execution, a unit is reported applied only when the object that
landed is exactly what the write installed — which is what the unit wants.
Kept, already satisfied, declared and delegated units never claim it. -/
theorem apply_applied_landed {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Report u d .Applied)) :
    s.phase = .Writing ∧ ∃ f i w l, s.unit = some f ∧ s.install = some i ∧
      Installs f.want w i ∧ e = .Landed l ∧ Rollback.Lands i l :=
  (run_post hrun h).2.2.1 _ _ rfl

/-- (FND-3) In every execution, a release is written only for a destination
that no deploy unit of the run claims — compared by the host's interned
destination identity — and only after releases began. -/
theorem apply_release_undeclared {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    {w : apply.Write}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Mutate u d x i w)) :
    ∃ f, s.unit = some f ∧
      (IsRelease f.want → ¬ Covered s.declared.val d f.slot ∧ s.releasing = true) := by
  obtain ⟨-, f, hf, -, -, -, -, hr⟩ := (run_post hrun h).2.1 _ _ _ _ _ rfl
  exact ⟨f, hf, hr⟩

/-- Once releases began, no deploy unit is accepted: it aborts the run. So a
release decided against the claims so far is never contradicted by a later
deploy. -/
theorem apply_no_deploy_after_release (s : apply.ApplyState) (f : apply.UnitFacts)
    (o a : recover.Ident)
    (hph : s.phase = .ReadingUnit) (hr : s.releasing = true) (hw : f.want = .Deploy o a) :
    apply.apply_step s (.UnitRead f) ⦃ p =>
      p.2 = .Abort (.Order s.index) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, releasing, mutated, commit⟩
  simp only at hph hr
  subst hph hr
  simp [apply.apply_step, apply.on_reading, apply.on_unit, hw, apply.refuse]

/-- In every execution, the flip is issued only after every unit read was
reported settled — applied, already there, kept, declared or delegated — and
after the publication of the admitted commit; it names that commit's
reserved selection. A refused unit aborts the run, so it never reaches the
flip. -/
theorem apply_flip_after_every_unit {s s' : apply.ApplyState}
    {cs : List apply.Command} {e : apply.Event} {sel : selection.SelectionIdentity}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Flip sel)) :
    ∃ f, s'.commit = some f ∧ sel = f.reserved ∧
      ∀ k : U32, k.val < s'.index.val → Reported k cs := by
  obtain ⟨f, hf, hs, -, -, -, hk⟩ := (run_post hrun h).2.2.2.1 _ rfl
  exact ⟨f, hf, hs, hk⟩

/-- (K6) In every execution, the flip names a selection whose transaction
differs from the one it replaces and is bound to the generation the run
allocated, above the durable floor and not yet published; the run was not
vacuous, and the `touched` fact it was judged on is the machine's own record
of whether a unit changed the disk. -/
theorem apply_flip_admitted {s s' : apply.ApplyState}
    {cs : List apply.Command} {e : apply.Event} {sel : selection.SelectionIdentity}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Flip sel)) :
    ∃ f, s'.commit = some f ∧ sel = f.reserved ∧ Publication.Admitted f ∧
      ¬ Publication.Vacuous f ∧ f.touched = s'.mutated := by
  obtain ⟨f, hf, hs, ha, hv, ht, -⟩ := (run_post hrun h).2.2.2.1 _ rfl
  exact ⟨f, hf, hs, ha, hv, ht⟩

/-- In every execution, a run that reported any unit applied never finishes
vacuous: a landed write always cuts a generation. -/
theorem apply_vacuous_untouched {s s' : apply.ApplyState}
    {cs : List apply.Command} {e : apply.Event}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Finish .Vacuous)) :
    ¬ Applied cs := by
  intro ha
  have hm := (run_inv hrun).applied ha
  have := (run_post hrun h).2.2.2.2 rfl
  simp [hm] at this

/-- A refusal decided for the unit under examination ends the run: the step
answers `Abort` with that refusal and the machine is aborted. -/
theorem apply_refused_aborts (s : apply.ApplyState) (f : apply.UnitFacts)
    (live : recover.Live) (r : apply.Refusal)
    (hph : s.phase = .Observing) (hu : s.unit = some f)
    (hd : apply.decide_apply s.index f live = ok (.Refuse r)) :
    apply.apply_step s (.Observed f.dest live) ⦃ p =>
      p.2 = .Abort r ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, releasing, mutated, commit⟩
  simp only at hph hu hd
  subst hph hu
  unfold apply.apply_step apply.on_observed
  simp only
  obtain ⟨b, hb, hb'⟩ := spec_exists_ok (DestId.same_spec f.dest f.dest)
  have htrue : b = true := hb'.mpr rfl
  rw [hb, Std.bind_ok]
  simp only [htrue, ↓reduceIte]
  rw [hd, Std.bind_ok]
  simp [apply.refuse]

/-- Nothing follows an abort: every event answers `Abort` again, so no
write, publication or flip is ever issued after a refusal. -/
theorem apply_aborted_terminal (s : apply.ApplyState) (e : apply.Event)
    (hph : s.phase = .Aborted) :
    apply.apply_step s e ⦃ p => p.2 = .Abort .Protocol ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, releasing, mutated, commit⟩
  simp only at hph
  subst hph
  simp [apply.apply_step, apply.refuse]

end Gripsack.Kernel.Apply
