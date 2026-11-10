import Extracted.GripsackKernel
import Kernel.Selection
import Kernel.Recover
import Kernel.Ownership
import Kernel.Rollback
import Kernel.Publication
import Kernel.Retention
import Kernel.Directory

/-!
# The apply machine (plan/0056 F2 steps 7 and 9)

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
* `decide_apply_clear_refused`: a clear (a removal a directory change needs)
  over anything but gripsack's own last write refuses the run.
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
  accepted after that; `apply_unit_out_of_stage_aborts` and
  `apply_directory_out_of_stage_aborts`: no item of an earlier stage is.
* The directory fence (step 9): `apply_write_unheld` — no whole-object
  write targets a destination the directory domain holds;
  `apply_directory_unclaimed` — a directory step is commanded only for a
  destination no deploy of the run claims, in stage order, and never for a
  hold; `apply_directory_settled_answers` — a step is reported settled only
  in answer to the host's settlement of that step;
  `apply_directory_refusal_aborts` — a step the host cannot settle ends the
  run.
* `apply_flip_after_every_unit`: over every run, the flip comes only after
  every unit read was reported settled (a refused unit aborts, so it never
  reaches the flip), after every directory step commanded was reported
  settled, and after the publication.
* (K6) `apply_flip_admitted`: over every run, the flip names the reserved
  selection of an admitted publication: a fresh transaction bound to the
  allocated generation, above the floor, not yet published, never for a
  vacuous run — and the run's `touched` fact is the machine's own record of
  whether it changed the disk.
* `apply_vacuous_untouched`: over every run, a run that reported any unit
  applied, or any directory step that changed the disk, never finishes
  vacuous.
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
  | .Clear r, .Replace, i => r = .Removed ∧ i = .Removed
  | .Clear _, .Create, _ => False
  | .Clear _, .TakeOver, _ => False

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
  | Clear r => cases r <;> cases live <;> cases recorded <;> cases t <;> cases drift <;> apply_cases

set_option maxHeartbeats 1600000 in
/-- The decision never reports a directory step: that is the directory
items' own report. -/
theorem decide_apply_not_directory (k : U32) (f : apply.UnitFacts) (live : recover.Live) :
    apply.decide_apply k f live ⦃ r => ∀ st, r = .Leave st → ∀ m, st ≠ .Directory m ⦄ := by
  obtain ⟨dest, slot, route, recorded, want, ⟨t⟩, drift⟩ := f
  cases want with
  | Deploy o a => cases live <;> cases recorded <;> cases t <;> cases drift <;> apply_cases
  | Release r => cases r <;> cases live <;> cases recorded <;> cases t <;> cases drift <;> apply_cases
  | Clear r => cases r <;> cases live <;> cases recorded <;> cases t <;> cases drift <;> apply_cases

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

/-- A clear empties only gripsack's own last write: over any other object it
refuses the run (the directory change cannot proceed), whatever the unit's
drift policy. -/
theorem decide_apply_clear_refused (k : U32) (f : apply.UnitFacts) (o : recover.Ident)
    (r : recover.Intended) (hw : f.want = .Clear r) (hc : f.consent.take_over = .Withheld)
    (hr : f.recorded ≠ .Wrote o) :
    apply.decide_apply k f (.Obj o) ⦃ x => ∃ why, x = .Refuse why ⦄ := by
  obtain ⟨dest, slot, route, recorded, want, ⟨t⟩, drift⟩ := f
  simp only at hw hc hr
  subst hw hc
  cases r <;> cases recorded <;> cases drift <;> apply_cases

/-! ## Release coverage (FND-3) and the directory domain's claims -/

/-- A deploy's claim covers a release at `d`: any deploy at the destination
covers a whole-object release; only the same block covers a block's. -/
def Covers (c : apply.Claim) (d : ids.DestId) : apply.Slot → Prop
  | .Whole => c.dest = d
  | .Block o => c.dest = d ∧ c.slot = .Block o

/-- Some claim in the list covers the release. -/
def Covered (cl : List apply.Claim) (d : ids.DestId) (sl : apply.Slot) : Prop :=
  ∃ c ∈ cl, Covers c d sl

/-- Some deploy of the run claims `d` (whole object or block). -/
def Claimed (cl : List apply.Claim) (d : ids.DestId) : Prop :=
  ∃ c ∈ cl, c.dest = d

def IsRelease : apply.Want → Prop
  | .Release _ => True
  | .Deploy _ _ => False
  | .Clear _ => False

def IsDeploy : apply.Want → Prop
  | .Deploy _ _ => True
  | .Release _ => False
  | .Clear _ => False

/-- The stage a unit belongs to. -/
def UnitStage : apply.Want → directory.Stage
  | .Clear _ => .Clearing
  | .Deploy _ _ => .Units
  | .Release _ => .Releases

theorem unit_stage_ok (w : apply.Want) : apply.unit_stage w = ok (UnitStage w) := by
  cases w <;> rfl

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

theorem claimed_loop_spec (cl : Slice apply.Claim) (d : ids.DestId) (i : Usize)
    (hi : i.val ≤ cl.val.length)
    (hprefix : ∀ j (h : j < cl.val.length), j < i.val → cl.val[j].dest ≠ d) :
    apply.claimed_loop cl d i ⦃ b => b ↔ Claimed cl.val d ⦄ := by
  unfold apply.claimed_loop
  apply loop.spec_decr_nat (fun i : Usize => cl.val.length - i.val)
    (fun i : Usize => i.val ≤ cl.val.length ∧
      ∀ j (h : j < cl.val.length), j < i.val → cl.val[j].dest ≠ d)
  · intro i ⟨hi, hprefix⟩
    unfold apply.claimed_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < cl.val.length := by scalar_tac
      step as ⟨ c, hc ⟩
      step with DestId.same_spec as ⟨ b, hb ⟩
      split_ifs with hsame
      · simp only [WP.spec_ok]
        exact ⟨fun _ => ⟨c, by rw [hc]; exact List.getElem_mem hlt', hb.mp hsame⟩, fun _ => by simp⟩
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
          exact fun h => hsame (hb.mpr h)
    · simp only [WP.spec_ok]
      refine ⟨fun h => (by cases h), ?_⟩
      rintro ⟨c, hc, hcd⟩
      obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hc
      exact absurd hcd (hprefix j hj (by scalar_tac))
  · exact ⟨hi, hprefix⟩

theorem claimed_spec (cl : Slice apply.Claim) (d : ids.DestId) :
    apply.claimed cl d ⦃ b => b ↔ Claimed cl.val d ⦄ :=
  claimed_loop_spec cl d 0#usize (by simp) (fun j _ hj => by simp at hj)

/-! ## What each step helper returns -/

/-- The commit facts the machine builds: the host's observation, and its own
record of whether the run changed the disk. -/
def factsOf (inp : apply.CommitInputs) (m : Bool) : publication.CommitFacts :=
  { previous := inp.previous, reserved := inp.reserved, allocated := inp.allocated,
    floor := inp.floor, unchanged := inp.unchanged, touched := m, «exists» := inp.exists }

theorem commit_facts_ok (inp : apply.CommitInputs) (m : Bool) :
    apply.commit_facts inp m = ok (factsOf inp m) := by
  simp [apply.commit_facts, factsOf]

/-- The step ended the run: an abort, with the index and the `mutated` flag
as they were. -/
def Aborts (s : apply.ApplyState) (p : apply.ApplyState × apply.Command) : Prop :=
  ∃ r, p.2 = .Abort r ∧ p.1.index = s.index ∧ p.1.mutated = s.mutated ∧ p.1.phase = .Aborted

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
  p = ({ s with unit := none, install := none, step := none, phase := .Aborted },
    .Abort .Malformed) ∨
  ∃ i : U32, i.val = s.index.val + 1 ∧
    p = ({ s with index := i, unit := none, install := none, step := none, phase := .ReadingUnit }, .ReadUnit i)

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

/-- Where a unit read takes the run: an abort, or — in stage order, as the
unit's stage — a deploy examined and claimed off the held destinations, a
release reported declared or held, or a release or clear examined off the
held destinations (a release, only unclaimed). -/
def OnUnit (f : apply.UnitFacts) (p : apply.ApplyState × apply.Command) : Prop :=
  Aborts s p ∨
  (Directory.Rank s.stage ≤ Directory.Rank (UnitStage f.want) ∧ (
    (IsDeploy f.want ∧ f.dest ∉ s.holds.val ∧ ∃ v : alloc.vec.Vec apply.Claim,
      v.val = s.declared.val ++ [⟨f.dest, f.slot⟩] ∧
      (p = ({ s with declared := v, stage := UnitStage f.want, phase := .Observing, unit := some f }, .Observe s.index f.dest) ∨
       p = ({ s with declared := v, stage := UnitStage f.want, phase := .Delegating, unit := some f }, .Delegate s.index f.dest))) ∨
    (IsRelease f.want ∧ ∃ st, (st = .Declared ∨ st = .Held) ∧
      p = ({ s with unit := some f, stage := UnitStage f.want, phase := .Reporting },
        .Report s.index f.dest st)) ∨
    (¬ IsDeploy f.want ∧ f.dest ∉ s.holds.val ∧
      (IsRelease f.want → ¬ Covered s.declared.val f.dest f.slot) ∧
      (p = ({ s with stage := UnitStage f.want, phase := .Observing, unit := some f },
          .Observe s.index f.dest) ∨
       p = ({ s with stage := UnitStage f.want, phase := .Delegating, unit := some f },
          .Delegate s.index f.dest)))))

theorem on_unit_spec (f : apply.UnitFacts) : apply.on_unit s f ⦃ p => OnUnit s f p ⦄ := by
  unfold apply.on_unit OnUnit
  rw [unit_stage_ok, Std.bind_ok]
  obtain ⟨b, hb, hbs⟩ := spec_exists_ok (Directory.ordered_spec s.stage (UnitStage f.want))
  rw [hb, Std.bind_ok]
  cases b
  · simp [apply.refuse, Aborts]
  have hrank := hbs.mp rfl
  simp only [↓reduceIte]
  cases hw : f.want with
  | Deploy o a =>
    simp only
    obtain ⟨b1, hb1, hh⟩ :=
      spec_exists_ok (Directory.names_spec (alloc.vec.Vec.deref s.holds) f.dest)
    rw [hb1, Std.bind_ok]
    simp only [deref_val] at hh
    cases b1
    · have hnh : f.dest ∉ s.holds.val := fun h => by simpa using hh.mpr h
      simp only [Bool.false_eq_true, ↓reduceIte]
      split_ifs with hmax
      · simp [apply.refuse, Aborts]
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
        right
        refine ⟨by rw [hw] at hrank; exact hrank, Or.inl ⟨by simp [IsDeploy], hnh, v, hv, ?_⟩⟩
        simpa [UnitStage] using hp
    · simp [apply.refuse, Aborts]
  | Release r =>
    simp only
    obtain ⟨b1, hb1, hcov⟩ :=
      spec_exists_ok (covered_spec (alloc.vec.Vec.deref s.declared) f.dest f.slot)
    rw [hb1, Std.bind_ok]
    simp only [deref_val] at hcov
    cases b1
    · have hnc : ¬ Covered s.declared.val f.dest f.slot := fun h => by simpa using hcov.mpr h
      simp only [Bool.false_eq_true, ↓reduceIte]
      obtain ⟨b2, hb2, hh⟩ :=
        spec_exists_ok (Directory.names_spec (alloc.vec.Vec.deref s.holds) f.dest)
      rw [hb2, Std.bind_ok]
      simp only [deref_val] at hh
      cases b2
      · have hnh : f.dest ∉ s.holds.val := fun h => by simpa using hh.mpr h
        simp only [Bool.false_eq_true, ↓reduceIte]
        refine WP.spec_mono (examine_spec _ f) ?_
        intro p hp
        right
        refine ⟨by rw [hw] at hrank; exact hrank, Or.inr (Or.inr ⟨by simp [IsDeploy], hnh,
          fun _ => hnc, ?_⟩)⟩
        simpa [UnitStage] using hp
      · simp only [↓reduceIte, apply.report, WP.spec_ok]
        right
        exact ⟨by rw [hw] at hrank; exact hrank,
          Or.inr (Or.inl ⟨by simp [IsRelease], .Held, by simp, by simp [UnitStage]⟩)⟩
    · simp only [↓reduceIte, apply.report, WP.spec_ok]
      right
      exact ⟨by rw [hw] at hrank; exact hrank,
        Or.inr (Or.inl ⟨by simp [IsRelease], .Declared, by simp, by simp [UnitStage]⟩)⟩
  | Clear r =>
    simp only
    obtain ⟨b1, hb1, hh⟩ :=
      spec_exists_ok (Directory.names_spec (alloc.vec.Vec.deref s.holds) f.dest)
    rw [hb1, Std.bind_ok]
    simp only [deref_val] at hh
    cases b1
    · have hnh : f.dest ∉ s.holds.val := fun h => by simpa using hh.mpr h
      simp only [Bool.false_eq_true, ↓reduceIte]
      refine WP.spec_mono (examine_spec _ f) ?_
      intro p hp
      right
      refine ⟨by rw [hw] at hrank; exact hrank, Or.inr (Or.inr ⟨by simp [IsDeploy], hnh,
        by simp [IsRelease], ?_⟩)⟩
      simpa [UnitStage] using hp
    · simp [apply.refuse, Aborts]

/-- Where a directory item takes the run: an abort, or — in stage order,
for a destination no deploy claims — the commanded step, or the recorded
hold. -/
def OnDirectory (f : directory.DirectoryFacts) (p : apply.ApplyState × apply.Command) : Prop :=
  Aborts s p ∨
  (Directory.Rank s.stage ≤ Directory.Rank (Directory.StageOf f.action) ∧
    ¬ Claimed s.declared.val f.dest ∧ (
    (Directory.Commands f.action ∧
      p = ({ s with phase := .Directing, stage := Directory.StageOf f.action, step := some f },
        .Directory s.index f.dest f.action)) ∨
    (¬ Directory.Commands f.action ∧ ∃ v : alloc.vec.Vec ids.DestId,
      v.val = s.holds.val ++ [f.dest] ∧
      p = ({ s with stage := Directory.StageOf f.action, holds := v, step := some f, phase := .Reporting }, .Report s.index f.dest .Held))))

theorem on_directory_spec (f : directory.DirectoryFacts) :
    apply.on_directory s f ⦃ p => OnDirectory s f p ⦄ := by
  unfold apply.on_directory OnDirectory
  rw [Directory.stage_ok, Std.bind_ok]
  obtain ⟨b, hb, hbs⟩ :=
    spec_exists_ok (Directory.ordered_spec s.stage (Directory.StageOf f.action))
  rw [hb, Std.bind_ok]
  cases b
  · simp [apply.refuse, Aborts]
  have hord := hbs.mp rfl
  simp only [↓reduceIte]
  obtain ⟨b1, hb1, hb1s⟩ := spec_exists_ok (claimed_spec (alloc.vec.Vec.deref s.declared) f.dest)
  rw [hb1, Std.bind_ok]
  simp only [deref_val] at hb1s
  cases b1
  · have hcl : ¬ Claimed s.declared.val f.dest := fun h => by simpa using hb1s.mpr h
    simp only [Bool.false_eq_true, ↓reduceIte]
    obtain ⟨b2, hb2, hb2s⟩ := spec_exists_ok (Directory.commands_spec f.action)
    rw [hb2, Std.bind_ok]
    cases b2
    · have hcmd : ¬ Directory.Commands f.action := fun h => by simpa using hb2s.mpr h
      simp only [Bool.false_eq_true, ↓reduceIte]
      split_ifs with hmax
      · simp [apply.refuse, Aborts]
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
        simp only [apply.report, WP.spec_ok]
        exact Or.inr ⟨hord, hcl, Or.inr ⟨hcmd, v, hv, by simp⟩⟩
    · simp only [↓reduceIte, WP.spec_ok]
      exact Or.inr ⟨hord, hcl, Or.inl ⟨hb2s.mp rfl, by simp⟩⟩
  · simp [apply.refuse, Aborts]

theorem on_observed_spec (d : ids.DestId) (live : recover.Live) :
    apply.on_observed s d live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f, s.unit = some f ∧ f.dest = d ∧
        ((∃ i w, WriteOk f live i w ∧
            p = ({ s with phase := .Writing, install := some i }, .Mutate s.index d live i w)) ∨
         (∃ st, st ≠ .Applied ∧ (∀ m, st ≠ .Directory m) ∧
            p = ({ s with phase := .Reporting }, .Report s.index d st))) ⦄ := by
  unfold apply.on_observed
  cases he : s.unit with
  | none => simp [apply.refuse, he]
  | some f =>
    simp only
    step with DestId.same_spec as ⟨ b, hb ⟩
    split_ifs with hd
    · have hfd : f.dest = d := hb.mp hd
      have hspec := decide_apply_spec s.index f live
      obtain ⟨r, hr, hw, hra⟩ := spec_exists_ok hspec
      rw [hr, Std.bind_ok]
      cases r with
      | Write i w =>
        simp only [WP.spec_ok]
        exact Or.inr ⟨f, rfl, hfd, Or.inl ⟨i, w, hw i w rfl, by simp⟩⟩
      | Leave st =>
        have hnd : ∀ m, st ≠ .Directory m := by
          obtain ⟨r', hr', hnd⟩ := spec_exists_ok (decide_apply_not_directory s.index f live)
          rw [hr] at hr'
          simp only [ok.injEq] at hr'
          subst hr'
          exact hnd st rfl
        simp only [apply.report, WP.spec_ok]
        exact Or.inr ⟨f, rfl, hfd, Or.inr ⟨st, fun h => hra (by rw [h]), hnd, by simp [he]⟩⟩
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

/-- A commanded directory step is answered: settled (changing the disk or
not) and reported as such, or refused, which aborts. -/
theorem on_directed_spec (e : apply.Event) :
    apply.on_directed s e ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f m, s.step = some f ∧ e = .Directed m ∧
        p = ({ s with mutated := (m || s.mutated), phase := .Reporting },
          .Report s.index f.dest (.Directory m)) ⦄ := by
  unfold apply.on_directed
  cases he : s.step with
  | none => simp [apply.refuse, he]
  | some f =>
    cases e with
    | Directed m => cases m <;> simp [apply.report]
    | Refused => simp [apply.refuse, he]
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

/-- Some unit was reported applied, or some directory step was reported
changing the disk, in `cs`. -/
def Applied (cs : List apply.Command) : Prop :=
  ∃ k d, apply.Command.Report k d .Applied ∈ cs ∨ apply.Command.Report k d (.Directory true) ∈ cs

/-- Every directory step commanded in `cs` was reported settled. -/
def DirectoriesSettled (cs : List apply.Command) : Prop :=
  ∀ k d a, apply.Command.Directory k d a ∈ cs → ∃ m, apply.Command.Report k d (.Directory m) ∈ cs

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

/-- A command that neither reports a change, commands a directory step, nor
reports one settled. -/
def Calm (c : apply.Command) : Prop :=
  (∀ k d, c ≠ .Report k d .Applied) ∧ (∀ k d m, c ≠ .Report k d (.Directory m)) ∧
    (∀ k d a, c ≠ .Directory k d a)

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
    ¬ Covered s.declared.val f.dest f.slot ∧ s.stage = .Releases
  /-- A unit under examination is not at a destination the directory domain
  holds. -/
  unheld : Examining s.phase → ∀ f, s.unit = some f → f.dest ∉ s.holds.val
  /-- Every directory step commanded was reported settled — except the one
  outstanding, or after an abort. -/
  directed : ∀ k d a, apply.Command.Directory k d a ∈ cs →
    (∃ m, apply.Command.Report k d (.Directory m) ∈ cs) ∨ s.phase = .Aborted ∨
    (s.phase = .Directing ∧ k = s.index ∧ ∃ f, s.step = some f ∧ f.dest = d)
  /-- A reported change set the `mutated` flag. -/
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
  (∀ u d x i w, c ≠ .Mutate u d x i w) ∧ Calm c ∧ (∀ sel, c ≠ .Flip sel) ∧ c ≠ .Finish .Vacuous

/-- What one step guarantees, given the invariant before it. -/
structure StepPost (s : apply.ApplyState) (e : apply.Event) (cs : List apply.Command)
    (p : apply.ApplyState × apply.Command) : Prop where
  inv : Inv p.1 (cs ++ [p.2])
  mutate : ∀ u d x i w, p.2 = .Mutate u d x i w →
    s.phase = .Observing ∧ ∃ f, s.unit = some f ∧ e = .Observed d x ∧ f.dest = d ∧
      u = s.index ∧ WriteOk f x i w ∧ d ∉ s.holds.val ∧
      (IsRelease f.want → ¬ Covered s.declared.val d f.slot ∧ s.stage = .Releases)
  applied : ∀ u d, p.2 = .Report u d .Applied →
    s.phase = .Writing ∧ ∃ f i w l, s.unit = some f ∧ s.install = some i ∧
      Installs f.want w i ∧ e = .Landed l ∧ Rollback.Lands i l
  directory : ∀ u d a, p.2 = .Directory u d a →
    s.phase = .ReadingUnit ∧ e = .DirectoryRead ⟨d, a⟩ ∧ u = s.index ∧
      ¬ Claimed s.declared.val d ∧ Directory.Commands a ∧
      Directory.Rank s.stage ≤ Directory.Rank (Directory.StageOf a)
  settled : ∀ u d m, p.2 = .Report u d (.Directory m) →
    s.phase = .Directing ∧ e = .Directed m ∧ u = s.index ∧ ∃ f, s.step = some f ∧ f.dest = d
  flip : ∀ sel, p.2 = .Flip sel →
    ∃ f, p.1.commit = some f ∧ sel = f.reserved ∧ Publication.Admitted f ∧
      ¬ Publication.Vacuous f ∧ f.touched = p.1.mutated ∧
      (∀ k : U32, k.val < p.1.index.val → Reported k cs) ∧ DirectoriesSettled cs
  vacuous : p.2 = .Finish .Vacuous → s.mutated = false

theorem StepPost.of_quiet {s e cs p} (hi : Inv p.1 (cs ++ [p.2])) (hq : Quiet p.2) :
    StepPost s e cs p :=
  ⟨hi, fun u d x i w h => absurd h (hq.1 u d x i w), fun u d h => absurd h (hq.2.1.1 u d),
    fun u d a h => absurd h (hq.2.1.2.2 u d a), fun u d m h => absurd h (hq.2.1.2.1 u d m),
    fun sel h => absurd h (hq.2.2.1 sel), fun h => absurd h hq.2.2.2⟩

theorem applied_append {cs : List apply.Command} {c : apply.Command}
    (hc : (∀ k d, c ≠ .Report k d .Applied) ∧ ∀ k d, c ≠ .Report k d (.Directory true))
    (h : Applied (cs ++ [c])) : Applied cs := by
  obtain ⟨k, d, hm⟩ := h
  rcases hm with hm | hm <;> rcases List.mem_append.mp hm with hm | hm
  · exact ⟨k, d, Or.inl hm⟩
  · simp only [List.mem_singleton] at hm
    exact absurd hm.symm (hc.1 k d)
  · exact ⟨k, d, Or.inr hm⟩
  · simp only [List.mem_singleton] at hm
    exact absurd hm.symm (hc.2 k d)

theorem Calm.applied {c : apply.Command} (h : Calm c) :
    (∀ k d, c ≠ .Report k d .Applied) ∧ ∀ k d, c ≠ .Report k d (.Directory true) :=
  ⟨h.1, fun k d => h.2.1 k d true⟩

/-- The directory steps commanded in `cs ++ [c]`, for a `c` that commands none. -/
theorem directory_append {cs : List apply.Command} {c : apply.Command}
    (hc : ∀ k d a, c ≠ .Directory k d a) {k d a} (h : apply.Command.Directory k d a ∈ cs ++ [c]) :
    apply.Command.Directory k d a ∈ cs := by
  rcases List.mem_append.mp h with h | h
  · exact h
  · simp only [List.mem_singleton] at h
    exact absurd h.symm (hc k d a)

/-- Directory steps are settled whenever no step is outstanding and the run
did not abort. -/
theorem Inv.settled_now {s : apply.ApplyState} {cs : List apply.Command} (h : Inv s cs)
    (h1 : s.phase ≠ .Directing) (h2 : s.phase ≠ .Aborted) : DirectoriesSettled cs := by
  intro k d a hm
  rcases h.directed k d a hm with hr | ha | ⟨hd, -⟩
  · exact hr
  · exact absurd ha h2
  · exact absurd hd h1

/-- With no step outstanding and the run not aborted, every directory step
commanded so far stays settled after a command that commands none. -/
theorem Inv.settled_calm {s : apply.ApplyState} {cs : List apply.Command} (h : Inv s cs)
    (h1 : s.phase ≠ .Directing) (h2 : s.phase ≠ .Aborted) {c : apply.Command}
    (hc : ∀ k d a, c ≠ .Directory k d a) :
    ∀ k d a, apply.Command.Directory k d a ∈ cs ++ [c] →
      ∃ m, apply.Command.Report k d (.Directory m) ∈ cs ++ [c] := by
  intro k d a hmem
  obtain ⟨m, hr⟩ := h.settled_now h1 h2 k d a (directory_append hc hmem)
  exact ⟨m, List.mem_append_left _ hr⟩

/-- A step that keeps the index, the flag and the commit, issues a calm
command, and moves to a phase that is neither reporting, writing, examining
nor directing — from one with no step outstanding, or into an abort — keeps
the invariant. -/
theorem Inv.quiet {s : apply.ApplyState} {cs : List apply.Command} (h : Inv s cs)
    (s' : apply.ApplyState) (c : apply.Command)
    (hi : s'.index = s.index) (hm : s'.mutated = s.mutated)
    (h1 : s'.phase ≠ .Reporting) (h2 : s'.phase ≠ .Writing) (h3 : ¬ Examining s'.phase)
    (h4 : Committing s'.phase → Committing s.phase ∧ s'.commit = s.commit)
    (hd : s'.phase = .Aborted ∨ (s.phase ≠ .Directing ∧ s.phase ≠ .Aborted ∧ s'.phase ≠ .Directing))
    (hc : Calm c) : Inv s' (cs ++ [c]) where
  reported := fun k hk => (h.reported k (hi ▸ hk)).mono c
  reporting := fun hr => absurd hr h1
  writing := fun hw => absurd hw h2
  unclaimed := fun he => absurd he h3
  unheld := fun he => absurd he h3
  directed := fun k d a hmem => by
    have hm' := directory_append hc.2.2 hmem
    rcases hd with ha | ⟨hn1, hn2, -⟩
    · exact Or.inr (Or.inl ha)
    · obtain ⟨m, hr⟩ := h.settled_now hn1 hn2 k d a hm'
      exact Or.inl ⟨m, List.mem_append_left _ hr⟩
  applied := fun ha => hm ▸ h.applied (applied_append hc.applied ha)
  committed := fun hc' => by
    obtain ⟨hc0, hcm⟩ := h4 hc'
    obtain ⟨f, hf, ha, hv, ht⟩ := h.committed hc0
    exact ⟨f, hcm ▸ hf, ha, hv, hm ▸ ht⟩

theorem post_abort {s : apply.ApplyState} {e cs} (h : Inv s cs)
    (s' : apply.ApplyState) (r : apply.Refusal)
    (hi : s'.index = s.index) (hm : s'.mutated = s.mutated) (ha : s'.phase = .Aborted) :
    StepPost s e cs (s', .Abort r) :=
  StepPost.of_quiet (h.quiet _ _ hi hm (by simp [ha]) (by simp [ha]) (by simp [Examining, ha])
    (fun hc => by simp [Committing, ha] at hc) (Or.inl ha) (by simp [Calm]))
    (by simp [Quiet, Calm])

theorem post_aborts {s : apply.ApplyState} {e cs p} (h : Inv s cs) (ha : Aborts s p) :
    StepPost s e cs p := by
  obtain ⟨r, h2, hi, hm, hph⟩ := ha
  obtain ⟨s', c⟩ := p
  simp only at h2 hi hm hph
  subst h2
  exact post_abort h s' r hi hm hph

/-- A step into reporting a unit (not applied, not a directory step) that
keeps the index, the flag and the outstanding nothing keeps the invariant. -/
theorem Inv.report {s : apply.ApplyState} {cs : List apply.Command} (h : Inv s cs)
    (s' : apply.ApplyState) (d : ids.DestId) (st : apply.Settled)
    (hi : s'.index = s.index) (hm : s'.mutated = s.mutated) (hph : s'.phase = .Reporting)
    (hs : s.phase ≠ .Directing ∧ s.phase ≠ .Aborted)
    (hst : st ≠ .Applied ∧ ∀ m, st ≠ .Directory m) :
    Inv s' (cs ++ [.Report s.index d st]) where
  reported := fun k hk => (h.reported k (hi ▸ hk)).mono _
  reporting := fun _ => ⟨d, st, by simp [hi]⟩
  writing := fun hw => by simp [hph] at hw
  unclaimed := fun he => by simp [Examining, hph] at he
  unheld := fun he => by simp [Examining, hph] at he
  directed := fun k d' a hmem => by
    have hm' := directory_append (by simp) hmem
    obtain ⟨m, hr⟩ := h.settled_now hs.1 hs.2 k d' a hm'
    exact Or.inl ⟨m, List.mem_append_left _ hr⟩
  applied := fun ha => hm ▸ h.applied (applied_append
    ⟨fun k d' e => by simp only [apply.Command.Report.injEq] at e; exact hst.1 e.2.2,
     fun k d' e => by simp only [apply.Command.Report.injEq] at e; exact hst.2 true e.2.2⟩ ha)
  committed := fun hc => by simp [Committing, hph] at hc

/-- A step into examining a unit, from reading it. -/
theorem Inv.examine {s : apply.ApplyState} {cs : List apply.Command} (h : Inv s cs)
    (s' : apply.ApplyState) (c : apply.Command) (f : apply.UnitFacts)
    (hi : s'.index = s.index) (hm : s'.mutated = s.mutated)
    (hph : s'.phase = .Observing ∨ s'.phase = .Delegating) (hu : s'.unit = some f)
    (hs : s.phase = .ReadingUnit) (hc : Calm c)
    (hr : IsRelease f.want → ¬ Covered s'.declared.val f.dest f.slot ∧ s'.stage = .Releases)
    (hh : f.dest ∉ s'.holds.val) : Inv s' (cs ++ [c]) :=
  { reported := fun k hk => (h.reported k (hi ▸ hk)).mono c
    reporting := fun hr' => by rcases hph with hp | hp <;> simp [hp] at hr'
    writing := fun hw => by rcases hph with hp | hp <;> simp [hp] at hw
    unclaimed := fun _ f' hf' hrel => by
      rw [hu] at hf'
      cases hf'
      exact hr hrel
    unheld := fun _ f' hf' => by
      rw [hu] at hf'
      cases hf'
      exact hh
    directed := fun k d a hmem => by
      have hm' := directory_append hc.2.2 hmem
      obtain ⟨m, hrep⟩ := h.settled_now (by simp [hs]) (by simp [hs]) k d a hm'
      exact Or.inl ⟨m, List.mem_append_left _ hrep⟩
    applied := fun ha => hm ▸ h.applied (applied_append hc.applied ha)
    committed := fun hc' => by rcases hph with hp | hp <;> simp [Committing, hp] at hc' }

theorem step_spec (s : apply.ApplyState) (e : apply.Event) (cs : List apply.Command)
    (h : Inv s cs) : apply.apply_step s e ⦃ p => StepPost s e cs p ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, stage, holds, step, mutated, commit⟩
  unfold apply.apply_step apply.on_reading
  cases phase <;> cases e
  all_goals try first
    | (with_reducible exact WP.spec_mono (failed_or_protocol_spec' _ _) (by rintro p ⟨r, rfl⟩; exact post_abort h _ r rfl rfl rfl))
    | (with_reducible exact WP.spec_mono (refuse_spec' _ _) (by rintro p rfl; exact post_abort h _ _ rfl rfl rfl))
  case Start.Start =>
    simp only [apply.read_unit, WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (by simp [Committing]) (Or.inr (by simp)) (by simp [Calm])) (by simp [Quiet, Calm])
  case ReadingUnit.UnitRead f =>
    refine WP.spec_mono (on_unit_spec _ f) ?_
    rintro p (ha | ⟨-, ⟨hd, hh, v, hv, rfl | rfl⟩ | ⟨hrel, st, hst, rfl⟩ | ⟨hnd, hh, hcov, rfl | rfl⟩⟩)
    · exact post_aborts h ha
    all_goals first
      | (refine StepPost.of_quiet (h.examine _ _ f rfl rfl (by simp) rfl rfl (by simp [Calm]) ?_ hh)
          (by simp [Quiet, Calm])
         intro hr'
         first
           | exact absurd hr' (by revert hd; cases f.want <;> simp [IsDeploy, IsRelease])
           | exact ⟨hcov hr', by revert hr'; cases f.want <;> simp [IsRelease, UnitStage]⟩)
      | (rcases hst with rfl | rfl <;>
         exact StepPost.of_quiet (h.report _ f.dest _ rfl rfl rfl (by simp) (by simp))
          (by simp [Quiet, Calm]))
  case ReadingUnit.DirectoryRead f =>
    refine WP.spec_mono (on_directory_spec _ f) ?_
    rintro p (ha | ⟨hord, hcl, ⟨hcmd, rfl⟩ | ⟨hcmd, v, hv, rfl⟩⟩)
    · exact post_aborts h ha
    · refine ⟨{ reported := fun k hk => (h.reported k hk).mono _
                reporting := by simp
                writing := by simp
                unclaimed := by simp [Examining]
                unheld := by simp [Examining]
                directed := ?_
                applied := fun ha => h.applied (applied_append (by simp) ha)
                committed := by simp [Committing] },
        by simp, by simp, ?_, by simp, by simp, by simp⟩
      · intro k d a hmem
        rcases List.mem_append.mp hmem with hm' | hm'
        · obtain ⟨m, hr⟩ := h.settled_now (by simp) (by simp) k d a hm'
          exact Or.inl ⟨m, List.mem_append_left _ hr⟩
        · simp only [List.mem_singleton, apply.Command.Directory.injEq] at hm'
          obtain ⟨rfl, rfl, rfl⟩ := hm'
          exact Or.inr (Or.inr ⟨rfl, rfl, f, rfl, rfl⟩)
      · intro u d a hc
        simp only [apply.Command.Directory.injEq] at hc
        obtain ⟨rfl, rfl, rfl⟩ := hc
        exact ⟨rfl, rfl, rfl, hcl, hcmd, hord⟩
    · exact StepPost.of_quiet (h.report _ f.dest .Held rfl rfl rfl (by simp) (by simp))
        (by simp [Quiet, Calm])
  case ReadingUnit.Exhausted =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (by simp [Committing]) (Or.inr (by simp)) (by simp [Calm])) (by simp [Quiet, Calm])
  case Observing.Observed d live =>
    refine WP.spec_mono (on_observed_spec _ d live) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, hf, hfd, ⟨i, w, hw, rfl⟩ | ⟨st, hst, hsd, rfl⟩⟩)
    · exact post_abort h _ r rfl rfl rfl
    · have hu := h.unclaimed (by simp [Examining]) f hf
      have hh := h.unheld (by simp [Examining]) f hf
      refine ⟨{ reported := fun k hk => (h.reported k hk).mono _
                reporting := by simp
                writing := fun _ => ⟨f, i, w, by simpa using hf, rfl, hw.1⟩
                unclaimed := ?_
                unheld := ?_
                directed := ?_
                applied := fun ha => h.applied (applied_append (by simp) ha)
                committed := by simp [Committing] }, ?_, by simp, by simp, by simp, by simp, by simp⟩
      · intro _ f' hf' hr'
        simp only at hf hf'
        rw [hf] at hf'
        cases hf'
        exact hu hr'
      · intro _ f' hf'
        simp only at hf hf'
        rw [hf] at hf'
        cases hf'
        exact hh
      · intro k d' a hmem
        have hm' := directory_append (by simp) hmem
        obtain ⟨m, hr⟩ := h.settled_now (by simp) (by simp) k d' a hm'
        exact Or.inl ⟨m, List.mem_append_left _ hr⟩
      · intro u d' x i' w' hm
        simp only [apply.Command.Mutate.injEq] at hm
        obtain ⟨rfl, rfl, rfl, rfl, rfl⟩ := hm
        exact ⟨rfl, f, hf, rfl, hfd, rfl, hw, hfd ▸ hh, fun hr => hfd ▸ hu hr⟩
    · exact StepPost.of_quiet (h.report _ d st rfl rfl rfl (by simp) ⟨hst, hsd⟩)
        (by simp [Quiet, Calm, hst, hsd])
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
      refine ⟨{ reported := fun k hk => (h.reported k hk).mono _
                reporting := fun _ => ⟨f.dest, .Applied, by simp⟩
                writing := by simp
                unclaimed := by simp [Examining]
                unheld := by simp [Examining]
                directed := ?_
                applied := fun _ => rfl
                committed := by simp [Committing] }, by simp, ?_, by simp, by simp, by simp, by simp⟩
      · intro k d' a hmem
        have hm' := directory_append (by simp) hmem
        obtain ⟨m, hr⟩ := h.settled_now (by simp) (by simp) k d' a hm'
        exact Or.inl ⟨m, List.mem_append_left _ hr⟩
      · intro u d hr
        exact ⟨rfl, f, i, w, live, hf, hi, hins, rfl, hl⟩
  case Writing.Failed failure =>
    exact WP.spec_mono (on_write_failed_spec _ failure)
      (by rintro p ⟨r, rfl⟩; exact post_abort h _ r rfl rfl rfl)
  case Reporting.Done =>
    refine WP.spec_mono (advance_spec _) ?_
    rintro p (rfl | ⟨i, hi, rfl⟩)
    · exact post_abort h _ _ rfl rfl rfl
    · refine StepPost.of_quiet ⟨fun k hk => ?_, by simp, by simp, by simp [Examining],
        by simp [Examining], ?_, fun ha => h.applied (applied_append (by simp) ha),
        by simp [Committing]⟩ (by simp [Quiet, Calm])
      · simp only at hk hi
        by_cases hlt : k.val < index.val
        · exact (h.reported k hlt).mono _
        · have : k = index := UScalar.eq_of_val_eq (by omega)
          subst this
          exact (h.reporting rfl).mono _
      · intro k d a hmem
        have hm' := directory_append (by simp) hmem
        obtain ⟨m, hr⟩ := h.settled_now (by simp) (by simp) k d a hm'
        exact Or.inl ⟨m, List.mem_append_left _ hr⟩
  case ReadingCommit.CommitRead inp =>
    refine WP.spec_mono (on_commit_spec _ inp) ?_
    rintro p (⟨r, rfl⟩ | ⟨hv, rfl⟩ | ⟨ha, hv, rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    · refine ⟨h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
        (by simp [Committing]) (Or.inr (by simp)) (by simp [Calm]), by simp, by simp, by simp,
        by simp, by simp, fun _ => ?_⟩
      exact by simpa [factsOf] using hv.2.2
    · refine StepPost.of_quiet ⟨fun k hk => (h.reported k hk).mono _, by simp, by simp,
        by simp [Examining], by simp [Examining],
        fun k d a hmem => Or.inl (h.settled_calm (by simp) (by simp) (by simp) k d a hmem),
        fun ha' => h.applied (applied_append (by simp) ha'),
        fun _ => ⟨_, rfl, ha, hv, rfl⟩⟩ (by simp [Quiet, Calm])
  case Publishing.Done =>
    refine WP.spec_mono (on_published_spec _) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, hf, rfl⟩)
    · exact post_abort h _ r rfl rfl rfl
    · obtain ⟨f', hf', ha, hv, ht⟩ := h.committed (by simp [Committing])
      simp only at hf hf'
      rw [hf] at hf'
      cases hf'
      refine ⟨h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
        (fun _ => ⟨by simp [Committing], rfl⟩) (Or.inr (by simp)) (by simp [Calm]),
        by simp, by simp, by simp, by simp, ?_, by simp⟩
      intro sel hsel
      simp only [apply.Command.Flip.injEq] at hsel
      exact ⟨f, hf, hsel.symm, ha, hv, ht, fun k hk => h.reported k hk,
        h.settled_now (by simp) (by simp)⟩
  case Flipping.Done =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (fun _ => ⟨by simp [Committing], rfl⟩) (Or.inr (by simp)) (by simp [Calm]))
      (by simp [Quiet, Calm])
  case Activating.Done =>
    simp only [WP.spec_ok]
    exact StepPost.of_quiet (h.quiet _ _ rfl rfl (by simp) (by simp) (by simp [Examining])
      (by simp [Committing]) (Or.inr (by simp)) (by simp [Calm])) (by simp [Quiet, Calm])
  all_goals first
    | (refine WP.spec_mono (on_delegated_spec _ _) ?_
       rintro p (⟨r, rfl⟩ | ⟨f, m, hf, he, rfl⟩)
       · exact post_abort h _ r rfl rfl rfl
       · refine StepPost.of_quiet ⟨fun k hk => (h.reported k hk).mono _,
           fun _ => ⟨f.dest, .Delegated m, by simp⟩, by simp, by simp [Examining],
           by simp [Examining], ?_, ?_, by simp [Committing]⟩ (by simp [Quiet, Calm])
         · intro k d a hmem
           have hm' := directory_append (by simp) hmem
           obtain ⟨m', hr⟩ := h.settled_now (by simp) (by simp) k d a hm'
           exact Or.inl ⟨m', List.mem_append_left _ hr⟩
         · intro ha
           have := h.applied (applied_append (by simp) ha)
           simp only at this
           simp [this])
    | (refine WP.spec_mono (on_directed_spec _ _) ?_
       rintro p (⟨r, rfl⟩ | ⟨f, m, hf, he, rfl⟩)
       · exact post_abort h _ r rfl rfl rfl
       · refine ⟨{ reported := fun k hk => (h.reported k hk).mono _
                   reporting := fun _ => ⟨f.dest, .Directory m, by simp⟩
                   writing := by simp
                   unclaimed := by simp [Examining]
                   unheld := by simp [Examining]
                   directed := ?_
                   applied := ?_
                   committed := by simp [Committing] },
           by simp, by simp, by simp, ?_, by simp, by simp⟩
         · intro k d a hmem
           rcases List.mem_append.mp hmem with hm' | hm'
           · rcases h.directed k d a hm' with ⟨m', hr⟩ | ha | ⟨-, hk, f', hf', hd⟩
             · exact Or.inl ⟨m', List.mem_append_left _ hr⟩
             · simp at ha
             · simp only at hf hf'
               rw [hf] at hf'
               cases hf'
               subst hk hd
               exact Or.inl ⟨m, by simp⟩
           · simp at hm'
         · intro ha
           obtain ⟨k, d, hm'⟩ := ha
           rcases hm' with hm' | hm' <;> rcases List.mem_append.mp hm' with hm' | hm'
           · have := h.applied ⟨k, d, Or.inl hm'⟩
             simp only at this
             simp [this]
           · simp at hm'
           · have := h.applied ⟨k, d, Or.inr hm'⟩
             simp only at this
             simp [this]
           · simp only [List.mem_singleton, apply.Command.Report.injEq,
               apply.Settled.Directory.injEq] at hm'
             simp [hm'.2.2]
         · intro u d m' hc
           simp only [apply.Command.Report.injEq, apply.Settled.Directory.injEq] at hc
           obtain ⟨rfl, rfl, rfl⟩ := hc
           exact ⟨rfl, he, rfl, f, hf, rfl⟩)

/-! ## The theorems -/

/-- Every execution keeps the invariant. -/
theorem run_inv {s : apply.ApplyState} {cs : List apply.Command} (h : Run s cs) : Inv s cs := by
  induction h with
  | start hs =>
    simp only [apply.ApplyState.new, ok.injEq] at hs
    subst hs
    exact ⟨fun k hk => by simp at hk, by simp, by simp, by simp [Examining], by simp [Examining],
      fun k d a hm => by simp at hm, fun ⟨k, d, hm⟩ => by simp at hm, by simp [Committing]⟩
  | step _ hstep ih => exact (spec_eq_ok (step_spec _ _ _ ih) hstep).inv

theorem run_post {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {c : apply.Command} (hrun : Run s cs)
    (h : apply.apply_step s e = ok (s', c)) : StepPost s e cs (s', c) :=
  spec_eq_ok (step_spec _ _ _ (run_inv hrun)) h

/-- (K2) Apply fidelity. In every execution, a write is issued only in
answer to the observation of the unit under decision, names that unit and
its destination, carries the observed object as `expect` (the driver
re-checks it before acting), and installs exactly what the unit wants: the
deploy's identity, the adoption's under consent, the release's, or a
clear's removal. -/
theorem apply_installs_intended {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    {w : apply.Write}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Mutate u d x i w)) :
    s.phase = .Observing ∧ ∃ f, s.unit = some f ∧ e = .Observed d x ∧ f.dest = d ∧
      u = s.index ∧ Installs f.want w i := by
  obtain ⟨hph, f, hf, he, hd, hu, hw, -⟩ := (run_post hrun h).mutate _ _ _ _ _ rfl
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
  obtain ⟨-, f, hf, -, -, -, ⟨-, ha⟩, -⟩ := (run_post hrun h).mutate _ _ _ _ _ rfl
  refine ⟨f, hf, ?_⟩
  cases w with
  | Create => exact Or.inl ha
  | Replace => exact Or.inr (Or.inl ha)
  | TakeOver => exact Or.inr (Or.inr ha)

/-- In every execution, a unit is reported applied only when the object that
landed is exactly what the write installed — which is what the unit wants.
Kept, already satisfied, declared, held, delegated and directory units never
claim it. -/
theorem apply_applied_landed {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Report u d .Applied)) :
    s.phase = .Writing ∧ ∃ f i w l, s.unit = some f ∧ s.install = some i ∧
      Installs f.want w i ∧ e = .Landed l ∧ Rollback.Lands i l :=
  (run_post hrun h).applied _ _ rfl

/-- (FND-3) In every execution, a release is written only for a destination
that no deploy unit of the run claims — compared by the host's interned
destination identity — and only after releases began. -/
theorem apply_release_undeclared {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    {w : apply.Write}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Mutate u d x i w)) :
    ∃ f, s.unit = some f ∧
      (IsRelease f.want → ¬ Covered s.declared.val d f.slot ∧ s.stage = .Releases) := by
  obtain ⟨-, f, hf, -, -, -, -, -, hr⟩ := (run_post hrun h).mutate _ _ _ _ _ rfl
  exact ⟨f, hf, hr⟩

/-- (Directory fence) In every execution, no whole-object write — deploy,
release or clear — targets a destination the directory domain holds. -/
theorem apply_write_unheld {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId} {x : recover.Live} {i : recover.Intended}
    {w : apply.Write}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Mutate u d x i w)) :
    d ∉ s.holds.val := by
  obtain ⟨-, f, -, -, -, -, -, hh, -⟩ := (run_post hrun h).mutate _ _ _ _ _ rfl
  exact hh

/-- (Directory fence) In every execution, a directory step is commanded only
when the host handed over that directory item, for a destination no deploy
unit of the run claims (whole object or block), for an action the host
executes (never a hold), and in stage order. -/
theorem apply_directory_unclaimed {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId} {a : directory.Action}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Directory u d a)) :
    e = .DirectoryRead ⟨d, a⟩ ∧ u = s.index ∧ ¬ Claimed s.declared.val d ∧
      Directory.Commands a ∧ Directory.Rank s.stage ≤ Directory.Rank (Directory.StageOf a) := by
  obtain ⟨-, he, hu, hc, hcmd, hr⟩ := (run_post hrun h).directory _ _ _ rfl
  exact ⟨he, hu, hc, hcmd, hr⟩

/-- (Directory fence) In every execution, a directory step is reported
settled only in answer to the host's settlement of the outstanding step, and
reports exactly whether the host said it changed the disk. -/
theorem apply_directory_settled_answers {s s' : apply.ApplyState} {cs : List apply.Command}
    {e : apply.Event} {u : U32} {d : ids.DestId} {m : Bool}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Report u d (.Directory m))) :
    s.phase = .Directing ∧ e = .Directed m ∧ u = s.index ∧ ∃ f, s.step = some f ∧ f.dest = d :=
  (run_post hrun h).settled _ _ _ rfl

/-- A directory step the host cannot settle ends the run: the step answers
`Abort` naming the item, and the machine is aborted. -/
theorem apply_directory_refusal_aborts (s : apply.ApplyState) (f : directory.DirectoryFacts)
    (hph : s.phase = .Directing) (hs : s.step = some f) :
    apply.apply_step s .Refused ⦃ p =>
      p.2 = .Abort (.Directory s.index f.dest) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, stage, holds, step, mutated, commit⟩
  simp only at hph hs
  subst hph hs
  simp [apply.apply_step, apply.on_directed, apply.refuse]

/-- Once releases began, no deploy unit is accepted: it aborts the run. So a
release decided against the claims so far is never contradicted by a later
deploy. -/
theorem apply_no_deploy_after_release (s : apply.ApplyState) (f : apply.UnitFacts)
    (o a : recover.Ident)
    (hph : s.phase = .ReadingUnit) (hr : s.stage = .Releases) (hw : f.want = .Deploy o a) :
    apply.apply_step s (.UnitRead f) ⦃ p =>
      p.2 = .Abort (.Order s.index) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, stage, holds, step, mutated, commit⟩
  simp only at hph hr
  subst hph hr
  simp [apply.apply_step, apply.on_reading, apply.on_unit, hw, apply.unit_stage,
    directory.ordered, directory.rank, apply.refuse]

/-- No unit of an earlier stage than the run's is accepted: it aborts. -/
theorem apply_unit_out_of_stage_aborts (s : apply.ApplyState) (f : apply.UnitFacts)
    (hph : s.phase = .ReadingUnit)
    (hr : Directory.Rank (UnitStage f.want) < Directory.Rank s.stage) :
    apply.apply_step s (.UnitRead f) ⦃ p =>
      p.2 = .Abort (.Order s.index) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, stage, holds, step, mutated, commit⟩
  simp only at hph hr
  subst hph
  unfold apply.apply_step apply.on_reading apply.on_unit
  simp only
  rw [unit_stage_ok, Std.bind_ok]
  obtain ⟨b, hb, hb'⟩ := spec_exists_ok (Directory.ordered_spec stage (UnitStage f.want))
  rw [hb, Std.bind_ok]
  have : b = false := by
    cases b
    · rfl
    · exact absurd (hb'.mp rfl) (by omega)
  simp [this, apply.refuse]

/-- No directory item of an earlier stage than the run's is accepted: it
aborts, so no step is commanded out of order. -/
theorem apply_directory_out_of_stage_aborts (s : apply.ApplyState)
    (f : directory.DirectoryFacts) (hph : s.phase = .ReadingUnit)
    (hr : Directory.Rank (Directory.StageOf f.action) < Directory.Rank s.stage) :
    apply.apply_step s (.DirectoryRead f) ⦃ p =>
      p.2 = .Abort (.Order s.index) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, stage, holds, step, mutated, commit⟩
  simp only at hph hr
  subst hph
  unfold apply.apply_step apply.on_reading apply.on_directory
  simp only
  rw [Directory.stage_ok, Std.bind_ok]
  obtain ⟨b, hb, hb'⟩ := spec_exists_ok (Directory.ordered_spec stage (Directory.StageOf f.action))
  rw [hb, Std.bind_ok]
  have : b = false := by
    cases b
    · rfl
    · exact absurd (hb'.mp rfl) (by omega)
  simp [this, apply.refuse]

/-- In every execution, the flip is issued only after every unit read was
reported settled — applied, already there, kept, declared, held, delegated
or a settled directory step —, after every directory step commanded was
reported settled by the host, and after the publication of the admitted
commit; it names that commit's reserved selection. A refused unit or a
refused directory step aborts the run, so it never reaches the flip. -/
theorem apply_flip_after_every_unit {s s' : apply.ApplyState}
    {cs : List apply.Command} {e : apply.Event} {sel : selection.SelectionIdentity}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Flip sel)) :
    ∃ f, s'.commit = some f ∧ sel = f.reserved ∧
      (∀ k : U32, k.val < s'.index.val → Reported k cs) ∧ DirectoriesSettled cs := by
  obtain ⟨f, hf, hs, -, -, -, hk, hd⟩ := (run_post hrun h).flip _ rfl
  exact ⟨f, hf, hs, hk, hd⟩

/-- (K6) In every execution, the flip names a selection whose transaction
differs from the one it replaces and is bound to the generation the run
allocated, above the durable floor and not yet published; the run was not
vacuous, and the `touched` fact it was judged on is the machine's own record
of whether a unit or a directory step changed the disk. -/
theorem apply_flip_admitted {s s' : apply.ApplyState}
    {cs : List apply.Command} {e : apply.Event} {sel : selection.SelectionIdentity}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Flip sel)) :
    ∃ f, s'.commit = some f ∧ sel = f.reserved ∧ Publication.Admitted f ∧
      ¬ Publication.Vacuous f ∧ f.touched = s'.mutated := by
  obtain ⟨f, hf, hs, ha, hv, ht, -, -⟩ := (run_post hrun h).flip _ rfl
  exact ⟨f, hf, hs, ha, hv, ht⟩

/-- In every execution, a run that reported any unit applied, or any
directory step that changed the disk, never finishes vacuous: it always cuts
a generation. -/
theorem apply_vacuous_untouched {s s' : apply.ApplyState}
    {cs : List apply.Command} {e : apply.Event}
    (hrun : Run s cs) (h : apply.apply_step s e = ok (s', .Finish .Vacuous)) :
    ¬ Applied cs := by
  intro ha
  have hm := (run_inv hrun).applied ha
  have := (run_post hrun h).vacuous rfl
  simp [hm] at this

/-- A refusal decided for the unit under examination ends the run: the step
answers `Abort` with that refusal and the machine is aborted. -/
theorem apply_refused_aborts (s : apply.ApplyState) (f : apply.UnitFacts)
    (live : recover.Live) (r : apply.Refusal)
    (hph : s.phase = .Observing) (hu : s.unit = some f)
    (hd : apply.decide_apply s.index f live = ok (.Refuse r)) :
    apply.apply_step s (.Observed f.dest live) ⦃ p =>
      p.2 = .Abort r ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, index, unit, install, declared, stage, holds, step, mutated, commit⟩
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
  rcases s with ⟨phase, index, unit, install, declared, stage, holds, step, mutated, commit⟩
  simp only at hph
  subst hph
  simp [apply.apply_step, apply.refuse]

end Gripsack.Kernel.Apply
