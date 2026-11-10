import Extracted.GripsackKernel
import Kernel.Selection

/-!
# Generation numbering and publication (plan/0056 K6, F2 step 5)

Theorems over the Aeneas translation of `gripsack_kernel::publication`
(`Extracted/GripsackKernel.lean`).

* `allocate_exceeds_floor`: a newly allocated generation number exceeds the
  current generation, the newest generation on disk and the durable
  high-water floor — a retired or rolled-back-from number is never reused.
* `floor_never_lowers`: the floor action keeps (seals) or raises the floor to
  cover every retained or about-to-be-published generation; it never moves it
  down.
* `publish_admitted`: an apply publishes only the number it allocated, above
  the durable floor, not already published, under a reserved selection whose
  transaction differs from the one the flip replaces (with `fresh_spec`,
  every flip carries a fresh transaction; rollback's flip is
  `Rollback.rollback_flip_admitted`).
* `vacuous_iff`: an apply is vacuous — publishes no generation and flips
  nothing — exactly when a previous generation exists, the state is unchanged
  and the run touched nothing on disk.

Rollback creates no generation: its machine has no publication command at all
(`Rollback.rollback_never_writes_generation`).
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.Publication

/-- `g` is at least every number `o` holds. -/
def AtLeast (g : generation.GenerationId) : Option generation.GenerationId → Prop
  | none => True
  | some h => h.number.val ≤ g.number.val

/-- `g` is strictly above every number `o` holds. -/
def Above (g : generation.GenerationId) : Option generation.GenerationId → Prop
  | none => True
  | some h => h.number.val < g.number.val

theorem later_spec (a b : Option generation.GenerationId) :
    publication.later a b ⦃ o =>
      (o = none ↔ a = none ∧ b = none) ∧
      ∀ g, o = some g → AtLeast g a ∧ AtLeast g b ∧ (a = some g ∨ b = some g) ⦄ := by
  cases a with
  | none => cases b <;> simp [publication.later, AtLeast]
  | some x =>
    cases b with
    | none => simp [publication.later, AtLeast]
    | some y =>
      simp only [publication.later, generation.GenerationId.value, Std.bind_ok]
      split_ifs with h
      · simp only [WP.spec_ok]
        refine ⟨by simp, fun g hg => ?_⟩
        cases hg
        exact ⟨by simp [AtLeast]; scalar_tac, by simp [AtLeast], Or.inr rfl⟩
      · simp only [WP.spec_ok]
        refine ⟨by simp, fun g hg => ?_⟩
        cases hg
        exact ⟨by simp [AtLeast], by simp [AtLeast]; scalar_tac, Or.inl rfl⟩

/-- (K6) A newly allocated generation number exceeds the current generation,
the newest generation on disk and the durable high-water floor. -/
theorem allocate_exceeds_floor (obs : publication.Allocation) :
    publication.allocate obs ⦃ r => ∀ g, r = some g →
      Above g obs.current ∧ Above g obs.newest ∧ Above g obs.floor ⦄ := by
  unfold publication.allocate
  step with later_spec as ⟨ o, ho1, ho2 ⟩
  step with later_spec as ⟨ o1, ho11, ho12 ⟩
  cases o1 with
  | none =>
    obtain ⟨hn, hf⟩ := ho11.mp rfl
    obtain ⟨hc, hw⟩ := ho1.mp hn
    simp [generation.GenerationId.new, hc, hw, hf, Above]
  | some top =>
    obtain ⟨htop1, htop2, -⟩ := ho12 top rfl
    simp only [generation.GenerationId.checked_next]
    split_ifs with hmax
    · simp
    · have hlt : top.number.val + 1 ≤ U64.max := by
        have hne : top.number.val ≠ core.num.U64.MAX.val :=
          fun he => hmax (UScalar.eq_of_val_eq he)
        have hb : top.number.val ≤ U64.max := by scalar_tac
        simp [core.num.U64.MAX, U64.rMax] at hne
        rw [U64.max_eq] at hb ⊢
        omega
      step as ⟨ n, hn ⟩
      simp only [Option.some.injEq]
      rintro g rfl
      have above : ∀ x, AtLeast top x → Above { number := n } x := by
        intro x hx
        cases x with
        | none => trivial
        | some x => simp only [AtLeast, Above] at hx ⊢; omega
      cases o with
      | none =>
        obtain ⟨hc, hw⟩ := ho1.mp rfl
        simp only [hc, hw]
        exact ⟨trivial, trivial, above _ htop2⟩
      | some m =>
        obtain ⟨hm1, hm2, -⟩ := ho2 m rfl
        have lift : ∀ x, AtLeast m x → AtLeast top x := by
          intro x hx
          cases x with
          | none => trivial
          | some x => simp only [AtLeast] at hx htop1 ⊢; omega
        exact ⟨above _ (lift _ hm1), above _ (lift _ hm2), above _ htop2⟩

/-- (K6) The floor never moves down: it is kept (and sealed) when it already
covers every retained generation, and otherwise raised to the larger of what
it says and the highest retained one — strictly above what it said. -/
theorem floor_never_lowers (observed retained : Option generation.GenerationId) :
    publication.floor_action observed retained ⦃ f =>
      (f = .Nothing ↔ observed = none ∧ retained = none) ∧
      (f = .Seal → ∃ o, observed = some o ∧ AtLeast o retained) ∧
      (∀ g, f = .Raise g → AtLeast g observed ∧ AtLeast g retained ∧
        ∀ o, observed = some o → o.number.val < g.number.val) ⦄ := by
  unfold publication.floor_action
  step with later_spec as ⟨ o, ho1, ho2 ⟩
  cases o with
  | none =>
    obtain ⟨h1, h2⟩ := ho1.mp rfl
    subst h1 h2
    simp
  | some floor =>
    obtain ⟨hf1, hf2, -⟩ := ho2 floor rfl
    cases observed with
    | none =>
      simp only [WP.spec_ok]
      refine ⟨by simp; intro hr; simp [hr] at ho1, by simp, ?_⟩
      intro g hg
      simp only [publication.FloorAction.Raise.injEq] at hg
      subst hg
      exact ⟨trivial, hf2, by simp⟩
    | some ob =>
      simp only
      step with GenerationId.same_spec as ⟨ b, hb ⟩
      split_ifs with h
      · have := hb.mp h
        subst this
        simp only [WP.spec_ok]
        exact ⟨by simp, fun _ => ⟨ob, rfl, hf2⟩, by simp⟩
      · simp only [WP.spec_ok]
        refine ⟨by simp, by simp, ?_⟩
        intro g hg
        simp only [publication.FloorAction.Raise.injEq] at hg
        subst hg
        have hne : ob ≠ floor := fun he => h (hb.mpr he)
        refine ⟨hf1, hf2, ?_⟩
        intro o ho
        simp only [Option.some.injEq] at ho
        subst ho
        simp only [AtLeast] at hf1
        have hv : ob.number.val ≠ floor.number.val := by
          intro hv
          apply hne
          obtain ⟨on⟩ := ob
          obtain ⟨fn⟩ := floor
          simp only at hv
          simp [UScalar.eq_of_val_eq hv]
        omega

/-- The decision to publish, as a proposition. -/
def Admitted (f : publication.CommitFacts) : Prop :=
  Fresh f.reserved f.previous ∧ (∃ t, f.reserved = .Transaction f.allocated t) ∧
    Above f.allocated f.floor ∧ f.exists = false

/-- The run is vacuous: a previous generation exists, the state is unchanged
and nothing on disk was touched. -/
def Vacuous (f : publication.CommitFacts) : Prop :=
  f.previous.isSome ∧ f.unchanged = true ∧ f.touched = false

theorem above_spec (g : generation.GenerationId) (fl : Option generation.GenerationId) :
    publication.above g fl ⦃ b => b ↔ Above g fl ⦄ := by
  cases fl <;> simp [publication.above, generation.GenerationId.value, Above]

theorem vacuous_spec (f : publication.CommitFacts) :
    publication.vacuous f ⦃ b => b ↔ Vacuous f ⦄ := by
  rcases f with ⟨previous, reserved, allocated, floor, unchanged, touched, ex⟩
  cases previous <;> cases unchanged <;> cases touched <;> simp [publication.vacuous, Vacuous]

/-- Publication is admitted only for the allocated number, above the floor,
not yet published, under a fresh reserved transaction bound to it. -/
theorem admit_publication_spec (f : publication.CommitFacts) :
    publication.admit_publication f ⦃ c => c ≠ .Vacuous ∧ (c = .Publish → Admitted f) ⦄ := by
  unfold publication.admit_publication
  step with fresh_spec as ⟨ b, hb ⟩
  by_cases h1 : b = true
  · obtain ⟨g, t, hres, -⟩ := hb h1
    simp only [h1, ↓reduceIte, hres, selection.SelectionIdentity.generation, Std.bind_ok]
    step with GenerationId.same_spec as ⟨ b1, hb1 ⟩
    by_cases h2 : b1 = true
    · simp only [h2, ↓reduceIte]
      step with above_spec as ⟨ b2, hb2 ⟩
      by_cases h3 : b2 = true
      · by_cases h4 : f.exists = true
        · simp [h3, h4]
        · simp only [h3, h4, ↓reduceIte, Bool.false_eq_true, WP.spec_ok]
          refine ⟨by simp, fun _ => ⟨hb h1, ⟨t, ?_⟩, hb2.mp h3, by simpa using h4⟩⟩
          rw [hres, hb1.mp h2]
      · simp [h3]
    · simp [h2]
  · simp [h1]

/-- (K6) An apply publishes only the generation it allocated, strictly above
the durable floor, under a name not yet published, and flips to a reserved
selection whose transaction differs from the one it replaces. A vacuous run
never publishes. -/
theorem publish_admitted (f : publication.CommitFacts) :
    publication.decide_commit f ⦃ c => c = .Publish → Admitted f ∧ ¬ Vacuous f ⦄ := by
  unfold publication.decide_commit
  step with vacuous_spec as ⟨ b, hb ⟩
  split_ifs with h
  · simp
  · refine WP.spec_mono (admit_publication_spec f) ?_
    rintro c ⟨-, hc⟩ hpub
    exact ⟨hc hpub, fun hv => h (hb.mpr hv)⟩

/-- (K6) Vacuous applies create no generation: the decision is `Vacuous` —
no publication, no flip — exactly when a previous generation exists, the
state is unchanged and the run touched nothing on disk. -/
theorem vacuous_iff (f : publication.CommitFacts) :
    publication.decide_commit f ⦃ c => c = .Vacuous ↔ Vacuous f ⦄ := by
  unfold publication.decide_commit
  step with vacuous_spec as ⟨ b, hb ⟩
  split_ifs with h
  · simp [hb.mp h]
  · refine WP.spec_mono (admit_publication_spec f) ?_
    rintro c ⟨hc, -⟩
    exact ⟨fun hv => absurd hv hc, fun hv => absurd (hb.mpr hv) h⟩

end Gripsack.Kernel.Publication
