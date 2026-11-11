import Extracted.GripsackKernel

/-!
# Update accounting and lock publication (plan/0056 F2 step 10)

Theorems over the Aeneas translation of `gripsack_kernel::update_survey`
(`Extracted/GripsackKernel.lean`): the former Verus `update-survey` family,
restated over the shipped code.

* `new_spec`: a fresh survey has counted nothing, is valid and satisfies the
  accounting invariant `Inv` (counted ≤ selected).
* `record_spec`: a report is admitted exactly when the survey is valid, the
  report is at the next position and entries remain; an admitted report is
  counted once, under its own disposition; a rejected one poisons the survey,
  which stays poisoned; the selection never changes and `Inv` is kept.
* `finish_spec`: a summary exists exactly for a valid survey that counted
  every selected entry and whose caller holds that many reports, and it
  carries the survey's counts.
* `finish_complete`: a summary's four counts add up to the selection — every
  selected entry was reported exactly once.
* `outcome_spec`: any failure makes the outcome `Incomplete`, whatever else
  changed; otherwise a change makes it `ChangesAvailable`.
* `publishes_lock_spec`: the lock is published only in publish mode, with no
  failed entry and at least one change.
* `lock_published_only_complete` (headline): for every survey a caller can
  hold (`Reach`: fresh, then any sequence of recorded reports), a summary
  accounts for every selected entry exactly once and a published lock comes
  from a survey with no failure and at least one change.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.UpdateSurvey

open update_survey

/-- Reports counted so far, as a natural number. -/
def counted (s : SurveyProgress) : Nat :=
  s.unchanged.val + s.changed.val + s.skipped.val + s.failed.val

/-- The accounting invariant: never more reports counted than selected. -/
def Inv (s : SurveyProgress) : Prop := counted s ≤ s.selected.val

/-- The count a disposition increments. -/
def bump (d : UpdateDisposition) (s : SurveyProgress) (n : Nat) : Prop :=
  match d with
  | .Unchanged => n = s.unchanged.val + 1
  | .Changed => n = s.changed.val + 1
  | .Skipped => n = s.skipped.val + 1
  | .Failed => n = s.failed.val + 1

/-- A fresh survey has counted nothing and is valid. -/
theorem new_spec (n : Usize) :
    SurveyProgress.new n ⦃ s =>
      s.selected = n ∧ counted s = 0 ∧ s.invalid = false ∧ Inv s ⦄ := by
  simp [SurveyProgress.new, counted, Inv]

theorem counted_spec (s : SurveyProgress) (h : Inv s) :
    SurveyProgress.counted s ⦃ c => c.val = counted s ⦄ := by
  unfold Inv counted at h
  have hmax : s.selected.val ≤ Usize.max := by scalar_tac
  unfold SurveyProgress.counted
  step as ⟨ i, hi ⟩
  step as ⟨ i1, hi1 ⟩
  step as ⟨ i2, hi2 ⟩
  simp [counted]
  omega

/-- The recording contract (former Verus `ensures`): admitted exactly at the
next position of a valid, unfinished survey; an admitted report increments
exactly its own disposition's count; a rejected one poisons the survey and
changes no count; the selection is kept and so is the invariant. -/
theorem record_spec (s : SurveyProgress) (h : Inv s) (reported : Usize)
    (d : UpdateDisposition) :
    SurveyProgress.record s reported d ⦃ admitted s' =>
      (admitted ↔ (s.invalid = false ∧ reported.val = counted s ∧ counted s < s.selected.val)) ∧
      s'.selected = s.selected ∧
      (s'.invalid = true ↔ ¬ admitted) ∧
      (admitted → counted s' = counted s + 1 ∧
        (match d with
         | .Unchanged => s'.unchanged.val = s.unchanged.val + 1 ∧ s'.changed = s.changed ∧
             s'.skipped = s.skipped ∧ s'.failed = s.failed
         | .Changed => s'.changed.val = s.changed.val + 1 ∧ s'.unchanged = s.unchanged ∧
             s'.skipped = s.skipped ∧ s'.failed = s.failed
         | .Skipped => s'.skipped.val = s.skipped.val + 1 ∧ s'.unchanged = s.unchanged ∧
             s'.changed = s.changed ∧ s'.failed = s.failed
         | .Failed => s'.failed.val = s.failed.val + 1 ∧ s'.unchanged = s.unchanged ∧
             s'.changed = s.changed ∧ s'.skipped = s.skipped)) ∧
      (¬ admitted → s'.unchanged = s.unchanged ∧ s'.changed = s.changed ∧
        s'.skipped = s.skipped ∧ s'.failed = s.failed) ∧
      Inv s' ⦄ := by
  unfold SurveyProgress.record
  step with counted_spec as ⟨ c, hc ⟩
  obtain ⟨sel, un, ch, sk, fa, inv⟩ := s
  simp only [Inv, counted] at h hc ⊢
  have hmax : sel.val ≤ Usize.max := by scalar_tac
  cases inv with
  | true => simp; omega
  | false =>
    simp only [Bool.false_eq_true, ↓reduceIte]
    split_ifs with h1 h2
    · simp only [bne_iff_ne, ne_eq] at h1
      have : reported.val ≠ c.val := fun e => h1 (UScalar.eq_of_val_eq e)
      simp; omega
    · simp only [bne_iff_ne, ne_eq, not_not] at h1
      subst h1 h2
      simp; omega
    · simp only [bne_iff_ne, ne_eq, not_not] at h1
      subst h1
      have hs : reported.val ≠ sel.val := fun e => h2 (UScalar.eq_of_val_eq e)
      cases d with
      | Unchanged =>
        step as ⟨ i, hi ⟩
        simp; omega
      | Changed =>
        step as ⟨ i, hi ⟩
        simp; omega
      | Skipped =>
        step as ⟨ i, hi ⟩
        simp; omega
      | Failed =>
        step as ⟨ i, hi ⟩
        simp; omega

/-- A summary exists exactly for a valid survey that counted every selected
entry, reported by a caller holding that many reports; it carries the
survey's counts. -/
theorem finish_spec (s : SurveyProgress) (h : Inv s) (reported : Usize) :
    SurveyProgress.finish s reported ⦃ r =>
      (r.isSome ↔ (s.invalid = false ∧ reported = s.selected ∧ counted s = s.selected.val)) ∧
      (∀ m, r = some m → m.counts = s) ⦄ := by
  unfold SurveyProgress.finish
  obtain ⟨sel, un, ch, sk, fa, inv⟩ := s
  cases inv with
  | true => simp
  | false =>
    simp only [Bool.false_eq_true, ↓reduceIte]
    by_cases hr : reported = sel
    · subst hr
      simp only [bne_self_eq_false, Bool.false_eq_true, ↓reduceIte]
      step with counted_spec as ⟨ c, hc ⟩
      by_cases hs : c = reported
      · subst hs
        simp_all
      · have hcs : c.val ≠ reported.val := fun e => hs (UScalar.eq_of_val_eq e)
        simp_all
    · simp [hr]

/-- (Lock publication) A summary accounts for every selected entry: its four
counts add up to the selection. -/
theorem finish_complete (s : SurveyProgress) (h : Inv s) (reported : Usize) :
    SurveyProgress.finish s reported ⦃ r => ∀ m, r = some m →
      m.counts.unchanged.val + m.counts.changed.val + m.counts.skipped.val +
        m.counts.failed.val = m.counts.selected.val ∧ m.counts.invalid = false ⦄ := by
  have hf := finish_spec s h reported
  unfold SurveyProgress.finish at hf ⊢
  obtain ⟨sel, un, ch, sk, fa, inv⟩ := s
  cases inv with
  | true => simp
  | false =>
    simp only [Bool.false_eq_true, ↓reduceIte] at hf ⊢
    split_ifs with h1
    · simp
    · step with counted_spec as ⟨ c, hc ⟩
      split_ifs with h2
      · simp
      · simp only [counted] at hc
        have : c.val = sel.val := by scalar_tac
        simp; omega

/-- A failure outranks a change. -/
theorem outcome_spec (m : UpdateSummary) :
    UpdateSummary.outcome m ⦃ o =>
      (o = .Incomplete ↔ m.counts.failed.val ≠ 0) ∧
      (o = .ChangesAvailable ↔ (m.counts.failed.val = 0 ∧ m.counts.changed.val ≠ 0)) ∧
      (o = .Current ↔ (m.counts.failed.val = 0 ∧ m.counts.changed.val = 0)) ⦄ := by
  unfold UpdateSummary.outcome
  by_cases hf : m.counts.failed = 0#usize
  · by_cases hc : m.counts.changed = 0#usize
    · simp [hf, hc]
    · have : m.counts.changed.val ≠ 0 := fun e => hc (by scalar_tac)
      simp [hf, hc, this]
  · have : m.counts.failed.val ≠ 0 := fun e => hf (by scalar_tac)
    simp [hf, this]

/-- (Lock publication) The lock is published only in publish mode, with no
failed entry and at least one change. -/
theorem publishes_lock_spec (m : UpdateSummary) (mode : UpdateMode) :
    UpdateSummary.publishes_lock m mode ⦃ p =>
      (p ↔ (mode = .Publish ∧ m.counts.failed.val = 0 ∧ m.counts.changed.val ≠ 0)) ⦄ := by
  unfold UpdateSummary.publishes_lock
  cases mode with
  | Check => simp
  | Publish =>
    by_cases hf : m.counts.failed = 0#usize
    · by_cases hc : m.counts.changed = 0#usize
      · simp [hf, hc]
      · have : m.counts.changed.val ≠ 0 := fun e => hc (by scalar_tac)
        simp [hf, hc, this]
    · have : m.counts.failed.val ≠ 0 := fun e => hf (by scalar_tac)
      simp [hf, this]

/-- The surveys a caller can hold: a fresh one, or one after any sequence of
recorded reports (admitted or not). -/
inductive Reach : SurveyProgress → Prop
  | new (n : Usize) (s : SurveyProgress) : SurveyProgress.new n = ok s → Reach s
  | record (s s' : SurveyProgress) (reported : Usize) (d : UpdateDisposition) (b : Bool) :
      Reach s → SurveyProgress.record s reported d = ok (b, s') → Reach s'

/-- Every survey a caller can hold satisfies the accounting invariant. -/
theorem reach_inv {s : SurveyProgress} (hs : Reach s) : Inv s := by
  induction hs with
  | new n s h =>
    have := new_spec n
    rw [h] at this
    simp only [WP.spec_ok] at this
    exact this.2.2.2
  | record s s' reported d b _ h ih =>
    have := record_spec s ih reported d
    rw [h] at this
    simp only [WP.spec_ok] at this
    exact this.2.2.2.2.2

/-- (Lock publication, headline) Whatever sequence of reports a caller
recorded, a summary it obtains accounts for every selected entry exactly once
— the four counts add up to the selection — and the lock it publishes comes
from a survey with no failed entry and at least one change. -/
theorem lock_published_only_complete {s : SurveyProgress} (hs : Reach s) (reported : Usize)
    (m : UpdateSummary) (hf : SurveyProgress.finish s reported = ok (some m)) :
    m.counts.unchanged.val + m.counts.changed.val + m.counts.skipped.val +
      m.counts.failed.val = m.counts.selected.val ∧
    m.counts.selected = s.selected ∧ reported = s.selected ∧
    (UpdateSummary.publishes_lock m .Publish = ok true →
      m.counts.failed.val = 0 ∧ m.counts.changed.val ≠ 0) := by
  have hinv := reach_inv hs
  have h1 := finish_spec s hinv reported
  have h2 := finish_complete s hinv reported
  rw [hf] at h1 h2
  simp only [WP.spec_ok] at h1 h2
  obtain ⟨hsome, hcounts⟩ := h1
  have hm := hcounts m rfl
  have hfin := hsome.mp rfl
  refine ⟨(h2 m rfl).1, by rw [hm], hfin.2.1, ?_⟩
  intro hp
  have h3 := publishes_lock_spec m .Publish
  rw [hp] at h3
  simp only [WP.spec_ok] at h3
  exact (h3.mp trivial).2

end Gripsack.Kernel.UpdateSurvey
