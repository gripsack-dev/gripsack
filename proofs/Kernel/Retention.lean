import Extracted.GripsackKernel
import Kernel.Selection

/-!
# GC retention admission (plan/0056 K4)

Theorems over the Aeneas translation of `gripsack_kernel::retention` and
`gripsack_kernel::generation` (`Extracted/GripsackKernel.lean`), relative to
the GC inventory the kernel is handed:

- nothing is planned while recovery or activation is pending, and a current
  generation missing from the inventory is refused (`admit_gc_spec`);
- pruning takes exactly the oldest excess of the inventory and never names
  the current generation (`plan_prune_spec`);
- deletions are exactly the candidate positions no root names
  (`plan_delete_spec`);
- inventory admission accepts exactly strictly ascending identities
  (`strictly_ascending_spec`, `GenerationList.new_spec`).
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel

instance : DecidableEq generation.GenerationId := fun a b =>
  decidable_of_iff (a.number = b.number) (by cases a; cases b; simp)

/-- `Vec::deref` exposes the vector's own list. -/
@[simp] theorem deref_val {α : Type} (v : alloc.vec.Vec α) :
    (alloc.vec.Vec.deref v).val = v.val := by
  simp [alloc.vec.Vec.deref, Slice.from_val]

@[simp] theorem deref_len {α : Type} (v : alloc.vec.Vec α) :
    (alloc.vec.Vec.deref v).len.val = v.val.length := by
  simp

/-- Numbers strictly increase from each position to the next. -/
def StrictlyAscending (ids : List generation.GenerationId) : Prop :=
  ∀ j (h : j < ids.length), 0 < j → ids[j - 1].number.val < ids[j].number.val

/-- `names_generation current g` is `current = some g`. -/
theorem names_generation_spec (current : Option generation.GenerationId)
    (g : generation.GenerationId) :
    generation.names_generation current g ⦃ r => r ↔ current = some g ⦄ := by
  cases current with
  | none => simp [generation.names_generation]
  | some c =>
    simp only [generation.names_generation]
    step with GenerationId.same_spec as ⟨ r, hr ⟩
    simp_all

/-- The ascent check from position `i`, knowing every earlier step ascends. -/
theorem strictly_ascending_loop_spec (ids : Slice generation.GenerationId) (i : Usize)
    (hi : 1 ≤ i.val)
    (hprefix : ∀ j (h : j < ids.val.length), 0 < j → j < i.val →
      ids.val[j - 1].number.val < ids.val[j].number.val) :
    generation.strictly_ascending_loop ids i ⦃ r => r ↔ StrictlyAscending ids.val ⦄ := by
  unfold generation.strictly_ascending_loop
  apply loop.spec_decr_nat (fun i : Usize => ids.val.length - i.val)
    (fun i : Usize => 1 ≤ i.val ∧ ∀ j (h : j < ids.val.length), 0 < j → j < i.val →
      ids.val[j - 1].number.val < ids.val[j].number.val)
  · intro i ⟨hi, hprefix⟩
    unfold generation.strictly_ascending_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < ids.val.length := by scalar_tac
      step as ⟨ i2, hi2 ⟩
      have hi2' : i2.val = i.val - 1 := hi2
      step as ⟨ x, hx ⟩
      step as ⟨ y, hy ⟩
      simp only [hi2'] at hx
      split_ifs with hge
      · simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
        intro hasc
        have := hasc i.val hlt' (by omega)
        rw [← hx, ← hy] at this
        scalar_tac
      · apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i3 hi3
        simp only [WP.spec_ok]
        and_intros
        all_goals try scalar_tac
        intro j hj hpos hji
        by_cases hlt'' : j < i.val
        · exact hprefix j hj hpos hlt''
        · have : j = i.val := by scalar_tac
          subst this
          rw [← hx, ← hy]
          scalar_tac
    · simp only [WP.spec_ok, true_iff]
      intro j hj hpos
      exact hprefix j hj hpos (by scalar_tac)
  · exact ⟨hi, hprefix⟩

/-- `strictly_ascending` decides `StrictlyAscending`. -/
theorem strictly_ascending_spec (ids : Slice generation.GenerationId) :
    generation.strictly_ascending ids ⦃ r => r ↔ StrictlyAscending ids.val ⦄ := by
  unfold generation.strictly_ascending
  apply strictly_ascending_loop_spec
  · simp
  · intro j _ hpos hj
    simp at hj
    omega

/-- Inventory admission: a list is admitted iff strictly ascending, and the
admitted inventory holds exactly the given identities. -/
theorem GenerationList.new_spec (ids : alloc.vec.Vec generation.GenerationId) :
    generation.GenerationList.new ids ⦃ r =>
      (r.isSome ↔ StrictlyAscending ids.val) ∧
      (∀ l, r = some l → l.ids = ids) ⦄ := by
  unfold generation.GenerationList.new
  apply WP.spec_bind (strictly_ascending_spec _)
  intro b hb
  split_ifs with h <;> simp_all [alloc.vec.Vec.deref, Slice.from_val]

/-- Membership from position `i`, knowing the needle is not before `i`. -/
theorem contains_generation_loop_spec (haystack : Slice generation.GenerationId)
    (needle : generation.GenerationId) (i : Usize)
    (hi : i.val ≤ haystack.val.length)
    (hprefix : ∀ j (h : j < haystack.val.length), j < i.val → haystack.val[j] ≠ needle) :
    retention.contains_generation_loop haystack needle i ⦃ r => r ↔ needle ∈ haystack.val ⦄ := by
  unfold retention.contains_generation_loop
  apply loop.spec_decr_nat (fun i : Usize => haystack.val.length - i.val)
    (fun i : Usize => i.val ≤ haystack.val.length ∧
      ∀ j (h : j < haystack.val.length), j < i.val → haystack.val[j] ≠ needle)
  · intro i ⟨hi, hprefix⟩
    unfold retention.contains_generation_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < haystack.val.length := by scalar_tac
      step as ⟨ x, hx ⟩
      apply WP.spec_bind (GenerationId.same_spec x needle)
      intro b hb
      split_ifs with hsame
      · simp only [WP.spec_ok, true_iff]
        rw [← hb.mp hsame, hx]
        exact List.getElem_mem _
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
          intro heq
          exact hsame (hb.mpr (by rw [hx, heq]))
    · simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
      have hlen : i.val = haystack.val.length := by scalar_tac
      intro hmem
      obtain ⟨j, hj, hjeq⟩ := List.getElem_of_mem hmem
      exact hprefix j hj (by omega) hjeq
  · exact ⟨hi, hprefix⟩

/-- `contains_generation` is membership. -/
theorem contains_generation_spec (haystack : Slice generation.GenerationId)
    (needle : generation.GenerationId) :
    retention.contains_generation haystack needle ⦃ r => r ↔ needle ∈ haystack.val ⦄ := by
  unfold retention.contains_generation
  apply contains_generation_loop_spec
  · simp
  · intro j _ hj; simp at hj

/-- K4 admission: recovery state wins; a current generation not in the
inventory is corruption; otherwise collection is admitted. -/
theorem admit_gc_spec (recovery_pending : Bool) (current : Option generation.GenerationId)
    (inventory : generation.GenerationList) :
    retention.admit_gc recovery_pending current inventory ⦃ r =>
      (r = .RecoveryPending ↔ recovery_pending) ∧
      (r = .CorruptCurrent ↔
        (¬ recovery_pending ∧ ∃ c, current = some c ∧ c ∉ inventory.ids.val)) ∧
      (r = .Admitted ↔
        (¬ recovery_pending ∧ ∀ c, current = some c → c ∈ inventory.ids.val)) ⦄ := by
  unfold retention.admit_gc
  cases recovery_pending
  · simp only [Bool.false_eq_true, ↓reduceIte]
    cases current with
    | none => simp
    | some c =>
      simp only [generation.GenerationList.as_slice, bind_ok]
      apply WP.spec_bind (contains_generation_spec (alloc.vec.Vec.deref inventory.ids) c)
      intro b hb
      split_ifs with h <;> simp_all [alloc.vec.Vec.deref, Slice.from_val]
  · simp

/-- The pruning loop from position `i`: the result is what was pruned so far
plus the non-current identities among positions `i` to `excess`. -/
theorem plan_prune_loop_spec (current : Option generation.GenerationId)
    (generations : Slice generation.GenerationId) (excess : Usize)
    (pruned : alloc.vec.Vec generation.GenerationId) (i : Usize)
    (hexcess : excess.val ≤ generations.val.length)
    (hi : i.val ≤ excess.val)
    (hpruned : pruned.val =
      (generations.val.take i.val).filter (fun g => decide (current ≠ some g))) :
    retention.plan_prune_loop current generations excess pruned i ⦃ v =>
      v.val = (generations.val.take excess.val).filter (fun g => decide (current ≠ some g)) ⦄ := by
  unfold retention.plan_prune_loop
  apply loop.spec_decr_nat
    (fun s : alloc.vec.Vec generation.GenerationId × Usize => excess.val - s.2.val)
    (fun s : alloc.vec.Vec generation.GenerationId × Usize => s.2.val ≤ excess.val ∧
      s.1.val = (generations.val.take s.2.val).filter (fun g => decide (current ≠ some g)))
  · rintro ⟨pruned, i⟩ ⟨hi, hpruned⟩
    dsimp only at hi hpruned
    unfold retention.plan_prune_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < excess.val := by scalar_tac
      have hig : i.val < generations.val.length := by omega
      step as ⟨ g, hg ⟩
      apply WP.spec_bind (names_generation_spec current g)
      intro b hb
      have hlen : pruned.val.length < Usize.max := by
        have := List.length_filter_le (fun g => decide (current ≠ some g))
          (generations.val.take i.val)
        have := generations.property
        rw [hpruned]
        scalar_tac
      have htake : generations.val.take (i.val + 1) = generations.val.take i.val ++ [g] := by
        rw [List.take_add_one, List.getElem?_eq_getElem hig, hg]
        rfl
      split_ifs with hcur
      · try simp only [bind_ok]
        apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i' hi'
        have hi'' : i'.val = i.val + 1 := by scalar_tac
        simp only [WP.spec_ok]
        and_intros
        all_goals try scalar_tac
        rw [hi'', htake, List.filter_append, hpruned]
        simp [hb.mp hcur]
      · apply WP.spec_bind (alloc.vec.Vec.push_spec pruned g hlen)
        intro pruned' hpruned'
        apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i' hi'
        have hi'' : i'.val = i.val + 1 := by scalar_tac
        simp only [WP.spec_ok]
        and_intros
        all_goals try scalar_tac
        rw [hi'', htake, List.filter_append, hpruned', hpruned]
        have : current ≠ some g := fun h => hcur (hb.mpr h)
        simp [this]
    · simp only [WP.spec_ok]
      have : i.val = excess.val := by scalar_tac
      rw [hpruned, this]
  · exact ⟨hi, hpruned⟩

/-- The excess the caller asked to prune: everything beyond `keep`. -/
def pruneExcess (length : Nat) (keep : Option Usize) : Nat :=
  match keep with
  | none => 0
  | some k => if k.val < length then length - k.val else 0

/-- K4 pruning: exactly the oldest `len - keep` identities of the inventory,
in order, minus the current generation. Hence every pruned identity is in
the inventory, the current generation is never pruned, and nothing newer
than the excess prefix is pruned. -/
theorem plan_prune_spec (inventory : generation.GenerationList)
    (current : Option generation.GenerationId) (keep : Option Usize) :
    retention.plan_prune inventory current keep ⦃ v =>
      v.val = (inventory.ids.val.take (pruneExcess inventory.ids.val.length keep)).filter
        (fun g => decide (current ≠ some g)) ⦄ := by
  unfold retention.plan_prune
  simp only [generation.GenerationList.as_slice, bind_ok]
  have hderef := deref_val inventory.ids
  cases keep with
  | none =>
    simp only [bind_ok, pruneExcess]
    rw [← hderef]
    exact plan_prune_loop_spec _ _ _ _ _ (by simp) (by simp) (by simp)
  | some k =>
    dsimp only
    split_ifs with hgt
    · rw [gt_iff_lt, UScalar.lt_equiv, deref_len] at hgt
      apply WP.spec_bind (Usize.sub_spec (by rw [deref_len]; omega))
      intro excess hexcess
      rw [deref_len] at hexcess
      have hE : pruneExcess inventory.ids.val.length (some k) = excess.val := by
        simp only [pruneExcess, hgt, ↓reduceIte]
        omega
      rw [hE, ← hderef]
      exact plan_prune_loop_spec _ _ _ _ _ (by rw [hderef]; omega) (by simp) (by simp)
    · rw [gt_iff_lt, UScalar.lt_equiv, deref_len, not_lt] at hgt
      have hE : pruneExcess inventory.ids.val.length (some k) = 0 := by
        simp only [pruneExcess, Nat.not_lt.mpr hgt, ↓reduceIte]
      simp only [bind_ok]
      rw [hE, ← hderef]
      exact plan_prune_loop_spec _ _ _ _ _ (by simp) (by simp) (by simp)

/-- The byte loop of `same_bytes` from position `i`: equality of two
equal-length slices, given the bytes before `i` agree. -/
theorem same_bytes_loop_spec (a b : Slice U8) (i : Usize)
    (hlen : a.val.length = b.val.length) (hi : i.val ≤ a.val.length)
    (hprefix : ∀ j, j < i.val → a.val[j]? = b.val[j]?) :
    retention.same_bytes_loop a b i ⦃ r => r ↔ a.val = b.val ⦄ := by
  unfold retention.same_bytes_loop
  apply loop.spec_decr_nat (fun i : Usize => a.val.length - i.val)
    (fun i : Usize => i.val ≤ a.val.length ∧ ∀ j, j < i.val → a.val[j]? = b.val[j]?)
  · intro i ⟨hi, hprefix⟩
    unfold retention.same_bytes_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < a.val.length := by scalar_tac
      step as ⟨ x, hx ⟩
      step as ⟨ y, hy ⟩
      split_ifs with hne
      · simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
        intro heq
        rw [bne_iff_ne] at hne
        apply hne
        rw [hx, hy]
        exact List.getElem_of_eq heq _
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
      have h : i.val = a.val.length := by scalar_tac
      apply List.ext_getElem?
      intro j
      by_cases hj : j < a.val.length
      · exact hprefix j (by omega)
      · rw [List.getElem?_eq_none (by scalar_tac), List.getElem?_eq_none (by scalar_tac)]
  · exact ⟨hi, hprefix⟩

/-- `same_bytes` is byte equality. -/
theorem same_bytes_spec (a b : Slice U8) :
    retention.same_bytes a b ⦃ r => r ↔ a.val = b.val ⦄ := by
  unfold retention.same_bytes
  dsimp only
  split_ifs with hne
  · simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
    intro h
    rw [bne_iff_ne] at hne
    apply hne
    apply UScalar.eq_of_val_eq
    simp [h]
  · apply same_bytes_loop_spec
    · rw [bne_iff_ne, not_not] at hne
      have := congrArg UScalar.val hne
      simpa [Slice.len_val] using this
    · simp
    · intro j hj; simp at hj

/-- Membership of a byte identity among owned identities, from position `i`. -/
theorem contains_bytes_loop_spec (haystack : Slice (alloc.vec.Vec U8)) (needle : Slice U8)
    (i : Usize) (hi : i.val ≤ haystack.val.length)
    (hprefix : ∀ j (h : j < haystack.val.length), j < i.val → haystack.val[j].val ≠ needle.val) :
    retention.contains_bytes_loop haystack needle i ⦃ r =>
      r ↔ ∃ v ∈ haystack.val, v.val = needle.val ⦄ := by
  unfold retention.contains_bytes_loop
  apply loop.spec_decr_nat (fun i : Usize => haystack.val.length - i.val)
    (fun i : Usize => i.val ≤ haystack.val.length ∧
      ∀ j (h : j < haystack.val.length), j < i.val → haystack.val[j].val ≠ needle.val)
  · intro i ⟨hi, hprefix⟩
    unfold retention.contains_bytes_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < haystack.val.length := by scalar_tac
      step as ⟨ v, hv ⟩
      apply WP.spec_bind (same_bytes_spec _ _)
      intro b hb
      split_ifs with hsame
      · simp only [WP.spec_ok, true_iff]
        refine ⟨v, ?_, ?_⟩
        · rw [hv]; exact List.getElem_mem _
        · have := hb.mp hsame
          simpa [alloc.vec.Vec.deref, Slice.from_val] using this
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
          intro heq
          apply hsame
          apply hb.mpr
          simp only [alloc.vec.Vec.deref, Slice.from_val]
          rw [hv]
          exact heq
    · simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
      have hlen : i.val = haystack.val.length := by scalar_tac
      rintro ⟨v, hmem, heq⟩
      obtain ⟨j, hj, hjeq⟩ := List.getElem_of_mem hmem
      exact hprefix j hj (by omega) (hjeq ▸ heq)
  · exact ⟨hi, hprefix⟩

/-- `contains_bytes` is membership by byte equality. -/
theorem contains_bytes_spec (haystack : Slice (alloc.vec.Vec U8)) (needle : Slice U8) :
    retention.contains_bytes haystack needle ⦃ r =>
      r ↔ ∃ v ∈ haystack.val, v.val = needle.val ⦄ := by
  unfold retention.contains_bytes
  apply contains_bytes_loop_spec
  · simp
  · intro j _ hj; simp at hj

/-- A candidate position no root names. -/
def Unreferenced (referenced candidates : Slice (alloc.vec.Vec U8)) (i : Nat) : Prop :=
  ∃ c, candidates.val[i]? = some c ∧ ∀ v ∈ referenced.val, v.val ≠ c.val

/-- The deletion loop from position `i`: the deletions so far are exactly the
unreferenced positions below `i`. -/
theorem plan_delete_loop_spec (referenced candidates : Slice (alloc.vec.Vec U8))
    (deletions : alloc.vec.Vec Usize) (i : Usize)
    (hi : i.val ≤ candidates.val.length)
    (hdel : ∀ d : Usize, d ∈ deletions.val ↔ (d.val < i.val ∧ Unreferenced referenced candidates d.val))
    (hlen : deletions.val.length ≤ i.val) :
    retention.plan_delete_loop referenced candidates deletions i ⦃ v =>
      ∀ d : Usize, d ∈ v.val ↔ Unreferenced referenced candidates d.val ⦄ := by
  unfold retention.plan_delete_loop
  apply loop.spec_decr_nat
    (fun s : alloc.vec.Vec Usize × Usize => candidates.val.length - s.2.val)
    (fun s : alloc.vec.Vec Usize × Usize => s.2.val ≤ candidates.val.length ∧
      (∀ d : Usize, d ∈ s.1.val ↔ (d.val < s.2.val ∧ Unreferenced referenced candidates d.val)) ∧
      s.1.val.length ≤ s.2.val)
  · rintro ⟨deletions, i⟩ ⟨hi, hdel, hlen⟩
    dsimp only at hi hdel hlen
    unfold retention.plan_delete_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < candidates.val.length := by scalar_tac
      step as ⟨ c, hc ⟩
      apply WP.spec_bind (contains_bytes_spec _ _)
      intro b hb
      have hci : candidates.val[i.val]? = some c := by
        rw [List.getElem?_eq_getElem hlt', hc]
      have hbderef : (b = true) ↔ ∃ v ∈ referenced.val, v.val = c.val := by
        simpa [alloc.vec.Vec.deref, Slice.from_val] using hb
      split_ifs with href
      · try simp only [bind_ok]
        apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i' hi'
        simp only [WP.spec_ok]
        and_intros
        all_goals try scalar_tac
        intro d
        rw [hdel]
        constructor
        · rintro ⟨hd, hu⟩; exact ⟨by scalar_tac, hu⟩
        · rintro ⟨hd, hu⟩
          refine ⟨?_, hu⟩
          by_contra hge
          have : d.val = i.val := by scalar_tac
          obtain ⟨c', hc', hall⟩ := hu
          rw [this, hci] at hc'
          cases hc'
          obtain ⟨v, hv, hveq⟩ := hbderef.mp href
          exact hall v hv hveq
      · have hdlen : deletions.val.length < Usize.max := by
          have := candidates.property
          scalar_tac
        apply WP.spec_bind (alloc.vec.Vec.push_spec deletions i hdlen)
        intro deletions' hdeletions'
        apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i' hi'
        simp only [WP.spec_ok]
        and_intros
        all_goals try scalar_tac
        · intro d
          rw [hdeletions', List.mem_append, List.mem_singleton, hdel]
          constructor
          · rintro (⟨hd, hu⟩ | hd)
            · exact ⟨by scalar_tac, hu⟩
            · subst hd
              refine ⟨by scalar_tac, c, hci, ?_⟩
              intro v hv heq
              exact href (hbderef.mpr ⟨v, hv, heq⟩)
          · rintro ⟨hd, hu⟩
            by_cases hlt'' : d.val < i.val
            · exact Or.inl ⟨hlt'', hu⟩
            · right
              have : d.val = i.val := by scalar_tac
              exact UScalar.eq_of_val_eq this
    · simp only [WP.spec_ok]
      have : i.val = candidates.val.length := by scalar_tac
      intro d
      rw [hdel]
      constructor
      · exact And.right
      · intro hu
        refine ⟨?_, hu⟩
        obtain ⟨c, hc, _⟩ := hu
        obtain ⟨hdlt, _⟩ := List.getElem?_eq_some_iff.mp hc
        omega
  · exact ⟨hi, hdel, hlen⟩

/-- K4 deletion: a position is deleted iff it is a candidate no root names —
deletions ⊆ candidates − roots, and nothing unreferenced survives. -/
theorem plan_delete_spec (referenced candidates : Slice (alloc.vec.Vec U8)) :
    retention.plan_delete referenced candidates ⦃ v =>
      ∀ d : Usize, d ∈ v.val ↔ Unreferenced referenced candidates d.val ⦄ := by
  unfold retention.plan_delete
  apply plan_delete_loop_spec
  · simp
  · intro d; simp
  · simp

end Gripsack.Kernel
