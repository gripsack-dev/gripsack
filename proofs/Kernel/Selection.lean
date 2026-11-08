import Extracted.GripsackKernel

/-!
# Commit classification — CLASSIFY-001 (plan/0056 K3)

Theorems over the Aeneas translation of `gripsack_kernel::selection`
(`Extracted/GripsackKernel.lean`): the kernel's identity functions are
equality, and `classify` satisfies the three biconditionals of the former
Verus contract — Committed iff current = target; Uncommitted iff fresh with
no current, or current = previous ≠ target; Ambiguous otherwise. Equality is
exact identity: generation number, pointer shape and every transaction byte.

Every theorem is a total-correctness Hoare triple `f args ⦃ r => P r ⦄`: the
call terminates without failure and its result satisfies `P`.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel

/-- `GenerationId::same` is equality. -/
theorem GenerationId.same_spec (a b : generation.GenerationId) :
    generation.GenerationId.same a b ⦃ r => r ↔ a = b ⦄ := by
  obtain ⟨a⟩ := a
  obtain ⟨b⟩ := b
  simp [generation.GenerationId.same]

/-- The byte loop of `TransactionId::same`, from position `i`: it answers
whether the two identities are equal, given that the bytes before `i` agree. -/
theorem TransactionId.same_loop_spec (a b : selection.TransactionId) (i : Usize)
    (hi : i.val ≤ 32)
    (hprefix : ∀ j, j < i.val → a.bytes.val[j]? = b.bytes.val[j]?) :
    selection.TransactionId.same_loop a b i ⦃ r => r ↔ a = b ⦄ := by
  unfold selection.TransactionId.same_loop
  apply loop.spec_decr_nat (fun i : Usize => 32 - i.val)
    (fun i : Usize => i.val ≤ 32 ∧ ∀ j, j < i.val → a.bytes.val[j]? = b.bytes.val[j]?)
  · intro i ⟨hi, hprefix⟩
    unfold selection.TransactionId.same_loop.body
    have hbytes : selection.TRANSACTION_ID_BYTES = 32#usize := by
      simp [selection.TRANSACTION_ID_BYTES]
    rw [hbytes]
    have ha : a.bytes.val.length = 32 := by simp
    have hb : b.bytes.val.length = 32 := by simp
    split_ifs with hlt
    · step as ⟨ x, hx ⟩
      step as ⟨ y, hy ⟩
      split_ifs with hne
      · simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
        intro heq
        subst heq
        have hxy : x = y := by rw [hx, hy]
        rw [bne_iff_ne] at hne
        exact hne hxy
      · rw [bne_iff_ne, not_not] at hne
        apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i' hi'
        simp only [WP.spec_ok]
        and_intros
        all_goals try scalar_tac
        intro j hj
        by_cases hji : j < i.val
        · exact hprefix j hji
        · have : j = i.val := by scalar_tac
          subst this
          rw [List.getElem?_eq_getElem (by scalar_tac), List.getElem?_eq_getElem (by scalar_tac),
            ← hx, ← hy, hne]
    · simp only [WP.spec_ok, true_iff]
      have h32 : i.val = 32 := by scalar_tac
      obtain ⟨abytes⟩ := a
      obtain ⟨bbytes⟩ := b
      congr
      apply Aeneas.Std.Array.ext
      apply List.ext_getElem?
      intro j
      by_cases hj : j < 32
      · exact hprefix j (by omega)
      · simp_all
  · exact ⟨hi, hprefix⟩

/-- `TransactionId::same` is byte-exact equality. -/
theorem TransactionId.same_spec (a b : selection.TransactionId) :
    selection.TransactionId.same a b ⦃ r => r ↔ a = b ⦄ := by
  unfold selection.TransactionId.same
  apply TransactionId.same_loop_spec
  · simp
  · intro j hj; simp at hj

/-- `SelectionIdentity::same` is equality: a legacy pointer never equals a
transaction pointer, and transactions compare generation and bytes. -/
theorem SelectionIdentity.same_spec (a b : selection.SelectionIdentity) :
    selection.SelectionIdentity.same a b ⦃ r => r ↔ a = b ⦄ := by
  cases a <;> cases b <;> simp only [selection.SelectionIdentity.same]
  · step with GenerationId.same_spec as ⟨ r, hr ⟩
    simp_all
  · simp
  · simp
  · step with GenerationId.same_spec as ⟨ r, hr ⟩
    split_ifs with hgen
    · step with TransactionId.same_spec as ⟨ r', hr' ⟩
      simp_all
    · simp_all

/-- Case analysis shared by the three CLASSIFY-001 biconditionals. -/
macro "classify_cases" facts:ident : tactic => `(tactic| (
  unfold selection.classify
  rcases $facts:ident with ⟨previous, target, current⟩
  cases current <;> cases previous
  case none.none => simp
  case none.some => simp
  case some.none =>
    step with SelectionIdentity.same_spec as ⟨ r, hr ⟩
    split_ifs with h1 <;> simp_all
  case some.some =>
    step with SelectionIdentity.same_spec as ⟨ r, hr ⟩
    split_ifs with h1
    · simp_all
    · step with SelectionIdentity.same_spec as ⟨ r', hr' ⟩
      split_ifs with h2 <;> simp_all))

/-- CLASSIFY-001 (i): Committed iff the live pointer is exactly the target. -/
theorem classify_committed_iff (facts : selection.RecoveryFacts) :
    selection.classify facts ⦃ c =>
      c = .Committed ↔ facts.current = some facts.target ⦄ := by
  classify_cases facts

/-- CLASSIFY-001 (ii): Uncommitted iff the run was fresh with no current
pointer, or the pointer is still the previous selection and not the target. -/
theorem classify_uncommitted_iff (facts : selection.RecoveryFacts) :
    selection.classify facts ⦃ c =>
      c = .Uncommitted ↔
        ((facts.current = none ∧ facts.previous = none) ∨
         (facts.previous.isSome ∧ facts.current = facts.previous ∧
          facts.current ≠ some facts.target)) ⦄ := by
  classify_cases facts

/-- CLASSIFY-001 (iii): Ambiguous otherwise — recovery changes nothing. -/
theorem classify_ambiguous_iff (facts : selection.RecoveryFacts) :
    selection.classify facts ⦃ c =>
      c = .Ambiguous ↔
        (facts.current ≠ some facts.target ∧
         ¬ (facts.current = none ∧ facts.previous = none) ∧
         ¬ (facts.previous.isSome ∧ facts.current = facts.previous)) ⦄ := by
  classify_cases facts

end Gripsack.Kernel
