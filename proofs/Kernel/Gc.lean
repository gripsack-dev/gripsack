import Extracted.GripsackKernel
import Kernel.Selection
import Kernel.Retention
import Kernel.Recover

/-!
# The GC machine (plan/0056 F2 step 4, K4 and K4′)

Theorems over the Aeneas translation of `gripsack_kernel::gc`
(`Extracted/GripsackKernel.lean`), relative to the `Inventory` event the
machine admitted (its `inventory`, `prune` and `keep` fields):

* `gc_effects_admitted` (K4): an effect — sealing the allocation floor,
  retiring a generation, the generations barrier, the delegated retirement,
  removing a candidate — is issued only in collect mode, with no recovery or
  activation pending and `current` in the inventory. `gc_preview_effect_free`:
  a preview issues none.
* `gc_retire_pruned` (K4): a retired generation is in the prune plan — so it
  is inventoried and is not `current` (`gc_never_retires_current`) — and no
  retained manifest names it as a predecessor; the driver re-checks `current`
  against the inventoried one.
* `gc_retire_newest_first`: retirements follow the inventory newest first,
  so an interrupted batch never strands a live receipt's predecessor.
* `gc_remove_unreferenced` (K4): a removed candidate is one the host
  inventoried, re-checked against its inventoried stamp, that no retained
  manifest pins and no external root names; `gc_doomed_exact`: the deletion set
  is exactly those candidates.
* `gc_retained_closed` (K4′): whenever an effect is issued, every inventoried
  generation's manifest was read, and every retained generation's
  predecessors are inventoried and retained — the roots are the closure of
  every retained manifest.
* `gc_remove_after_retirement`: every removal follows the retirement of every
  retired generation, the barrier on `generations/`, and the delegated
  retirement of their receipts.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.Gc

/-- The inventoried generations. -/
abbrev gens (s : gc.GcState) : List generation.GenerationId := s.inventory.generations.ids.val

/-- The prune plan `plan_prune` computes (`plan_prune_spec`). -/
def PrunePlan (inv : List generation.GenerationId) (current : Option generation.GenerationId)
    (keep : Option Usize) : List generation.GenerationId :=
  (inv.take (pruneExcess inv.length keep)).filter (fun g => decide (current ≠ some g))

theorem PrunePlan.mem {inv current keep g} (h : g ∈ PrunePlan inv current keep) :
    g ∈ inv ∧ current ≠ some g := by
  simp only [PrunePlan, List.mem_filter, decide_eq_true_eq] at h
  exact ⟨List.mem_of_mem_take h.1, h.2⟩

/-! ## What each step helper returns -/

section helpers
variable (s : gc.GcState)

theorem refuse_spec' (r : gc.Refusal) :
    gc.refuse s r ⦃ p => p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  simp [gc.refuse]

theorem failed_or_protocol_spec' (e : gc.Event) :
    gc.failed_or_protocol s e ⦃ p => ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold gc.failed_or_protocol
  cases e with
  | Failed f => cases f <;> simp [gc.refuse]
  | _ => simp [gc.refuse]

/-- Where `next_manifest` takes the run from `s`: the next older manifest,
or — all read — a closed decision and the external roots. -/
def NextManifest (p : gc.GcState × gc.Command) : Prop :=
  (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
  (0 < s.index.val ∧ ∃ g, (gens s)[s.index.val - 1]? = some g ∧
     p = ({ s with phase := .ReadingManifests }, .ReadManifest g)) ∨
  (s.index.val = 0 ∧ (∀ m ∈ s.manifests.val, Closes s.manifests.val m (gens s)) ∧
     p = ({ s with phase := .ReadingRoots }, .ReadRoots))

theorem next_manifest_spec (hi : s.index.val ≤ (gens s).length) :
    gc.next_manifest s ⦃ p => NextManifest s p ⦄ := by
  unfold gc.next_manifest NextManifest
  try simp only [generation.GenerationList.as_slice, bind_ok]
  split_ifs with h
  · have h0 : 0 < s.index.val := by scalar_tac
    step as ⟨ i, hi' ⟩
    have hlt : i.val < (gens s).length := by scalar_tac
    have hlt2 : i.val < (alloc.vec.Vec.deref s.inventory.generations.ids).val.length := by
      simpa using hlt
    step as ⟨ g, hg ⟩
    right; left
    refine ⟨h0, g, ?_, rfl⟩
    rw [List.getElem?_eq_getElem (by scalar_tac)]
    simp only [deref_val] at hg
    have : s.index.val - 1 = i.val := by scalar_tac
    simp [this, hg]
  · have h0 : s.index.val = 0 := by scalar_tac
    step with closure_spec as ⟨ o, ho ⟩
    simp only [deref_val] at ho
    cases o with
    | none =>
      try simp only [WP.spec_ok]
      right; right
      exact ⟨h0, ho rfl, by simp⟩
    | some u => cases u <;> simp [gc.refuse]

theorem fate_spec (prune : Slice generation.GenerationId) (ms : Slice retention.Manifest)
    (g : generation.GenerationId) :
    gc.fate prune ms g ⦃ f => f = .Retired → g ∈ prune.val ⦄ := by
  unfold gc.fate
  step with contains_generation_spec as ⟨ b, hb ⟩
  split_ifs with h
  · step with required_spec as ⟨ b1, hb1 ⟩
    split_ifs <;> simp [hb.mp h]
  · simp

theorem on_inventory_spec (f : gc.InventoryFacts) :
    gc.on_inventory s f ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      (f.recovery_pending = false ∧ (∀ c, f.current = some c → c ∈ f.generations.ids.val) ∧
        ∃ (v : alloc.vec.Vec generation.GenerationId) (i : Usize),
          v.val = PrunePlan f.generations.ids.val f.current s.keep ∧
          i.val = f.generations.ids.val.length ∧
          NextManifest { s with inventory := f, prune := v, index := i } p) ⦄ := by
  unfold gc.on_inventory
  step with admit_gc_spec as ⟨ a, h1, h2, h3 ⟩
  cases a with
  | Admitted =>
    have ⟨hp, hc⟩ := h3.mp rfl
    simp only
    step with plan_prune_spec as ⟨ v, hv ⟩
    simp only [generation.GenerationList.as_slice, bind_ok]
    refine WP.spec_mono (next_manifest_spec _ (by simp)) ?_
    intro p hp'
    right
    refine ⟨by simpa using hp, hc, v, _, hv, by simp, hp'⟩
  | RecoveryPending => simp [gc.refuse]
  | CorruptCurrent => simp [gc.refuse]

theorem on_manifest_spec (f : retention.ManifestFacts)
    (hlen : s.manifests.val.length + s.index.val = (gens s).length) :
    gc.on_manifest s f ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      (0 < s.index.val ∧ (gens s)[s.index.val - 1]? = some f.id ∧
        ∃ (fate : retention.Fate) (v : alloc.vec.Vec retention.Manifest) (i : Usize),
          (fate = .Retired → f.id ∈ s.prune.val) ∧
          v.val = s.manifests.val ++ [{ facts := f, fate := fate }] ∧
          i.val = s.index.val - 1 ∧
          NextManifest { s with manifests := v, index := i } p) ⦄ := by
  unfold gc.on_manifest
  split_ifs with h0
  · simp [gc.refuse]
  · have hpos : 0 < s.index.val := by scalar_tac
    step as ⟨ i, hi ⟩
    simp only [generation.GenerationList.as_slice, bind_ok]
    have hlt : i.val < (gens s).length := by scalar_tac
    have hlt2 : i.val < (alloc.vec.Vec.deref s.inventory.generations.ids).val.length := by
      simpa using hlt
    step as ⟨ g, hg ⟩
    step with GenerationId.same_spec as ⟨ b, hb ⟩
    split_ifs with hs
    · step with fate_spec as ⟨ fate, hfate ⟩
      simp only [deref_val] at hfate
      have hpush : s.manifests.val.length < Usize.max := by
        have := s.inventory.generations.ids.slice.property
        simp only [alloc.vec.Vec.val] at hlen ⊢
        scalar_tac
      apply WP.spec_bind (alloc.vec.Vec.push_spec _ _ hpush)
      intro v hv
      refine WP.spec_mono (next_manifest_spec _ (by show i.val ≤ (gens s).length; scalar_tac)) ?_
      intro p hp
      right
      have hid : f.id = g := hb.mp hs
      have hidx : s.index.val - 1 = i.val := by scalar_tac
      refine ⟨hpos, ?_, fate, v, i, hfate, hv, by scalar_tac, hp⟩
      rw [hidx, List.getElem?_eq_getElem hlt]
      simp only [deref_val] at hg
      simp [hid, hg]
    · simp [gc.refuse]

theorem on_candidates_spec (cands : alloc.vec.Vec retention.Candidate) :
    gc.on_candidates s cands ⦃ p =>
      ∃ v : alloc.vec.Vec Usize,
        (∀ d : Usize, d ∈ v.val ↔ Unreferenced s.manifests.val s.external.val cands.val d.val) ∧
        p = ({ s with phase := .Planning, candidates := cands, doomed := v }, .Plan) ⦄ := by
  unfold gc.on_candidates
  step with plan_delete_spec as ⟨ v, hv ⟩
  try simp only [deref_val] at hv
  exact ⟨v, hv, rfl⟩

/-- The first retired manifest at or after `i`, knowing none between `from`
and `i` is retired. -/
theorem next_retired_loop_spec (ms : Slice retention.Manifest) (i : Usize) (fr : Nat)
    (hfr : fr ≤ i.val) (hi : i.val ≤ ms.val.length)
    (hprefix : ∀ j (h : j < ms.val.length), fr ≤ j → j < i.val → ms.val[j].fate = .Retained) :
    gc.next_retired_loop ms i ⦃ o =>
      (∀ k : Usize, o = some k → fr ≤ k.val ∧ ∃ m, ms.val[k.val]? = some m ∧ m.fate = .Retired ∧
        ∀ j (h : j < ms.val.length), fr ≤ j → j < k.val → ms.val[j].fate = .Retained) ∧
      (o = none → ∀ j (h : j < ms.val.length), fr ≤ j → ms.val[j].fate = .Retained) ⦄ := by
  unfold gc.next_retired_loop
  apply loop.spec_decr_nat (fun i : Usize => ms.val.length - i.val)
    (fun i : Usize => fr ≤ i.val ∧ i.val ≤ ms.val.length ∧
      ∀ j (h : j < ms.val.length), fr ≤ j → j < i.val → ms.val[j].fate = .Retained)
  · intro i ⟨hfr, hi, hprefix⟩
    unfold gc.next_retired_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < ms.val.length := by scalar_tac
      step as ⟨ m, hm ⟩
      step with Fate.retained_spec as ⟨ b, hb ⟩
      split_ifs with hr
      · apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i2 hi2
        try simp only [WP.spec_ok]
        and_intros
        all_goals try scalar_tac
        intro j hj hfj hji
        by_cases hlt'' : j < i.val
        · exact hprefix j hj hfj hlt''
        · have : j = i.val := by scalar_tac
          subst this
          rw [← hm]
          exact hb.mp hr
      · simp only [WP.spec_ok, Option.some.injEq, reduceCtorEq, false_imp_iff, and_true]
        rintro k rfl
        refine ⟨hfr, m, by rw [List.getElem?_eq_getElem hlt', hm], ?_, hprefix⟩
        cases hf : m.fate
        · exact absurd (hb.mpr hf) hr
        · rfl
    · simp only [WP.spec_ok, reduceCtorEq, false_imp_iff, implies_true, true_and]
      intro _ j hj hfj
      exact hprefix j hj hfj (by scalar_tac)
  · exact ⟨hfr, hi, hprefix⟩

/-- Where `retire_next` takes the run: the next retired generation at or
after the cursor, or the barrier once none is left. -/
def RetireNext (p : gc.GcState × gc.Command) : Prop :=
  (∃ k : Usize, s.index.val ≤ k.val ∧ ∃ m, s.manifests.val[k.val]? = some m ∧ m.fate = .Retired ∧
     (∀ j (h : j < s.manifests.val.length), s.index.val ≤ j → j < k.val →
        s.manifests.val[j].fate = .Retained) ∧
     p = ({ s with phase := .Retiring, index := k }, .Retire m.facts.id s.inventory.current)) ∨
  ((∀ j (h : j < s.manifests.val.length), s.index.val ≤ j → s.manifests.val[j].fate = .Retained) ∧
     p = ({ s with phase := .Sealing }, .Barrier))

theorem retire_next_spec (hi : s.index.val ≤ s.manifests.val.length) :
    gc.retire_next s ⦃ p => RetireNext s p ⦄ := by
  unfold gc.retire_next RetireNext
  apply WP.spec_bind (next_retired_loop_spec _ s.index s.index.val le_rfl (by simpa using hi)
    (fun j _ h1 h2 => absurd h2 (by omega)))
  rintro o ⟨hsome, hnone⟩
  cases o with
  | none =>
    try simp only [WP.spec_ok]
    right
    exact ⟨by simpa using hnone rfl, by simp⟩
  | some k =>
    obtain ⟨hk, m, hm, hr, hpre⟩ := hsome k rfl
    try simp only [deref_val] at hm hpre
    have hlt : k.val < s.manifests.val.length := (List.getElem?_eq_some_iff.mp hm).1
    simp only
    step as ⟨ m', hm' ⟩
    have : m' = m := by
      rw [List.getElem?_eq_getElem hlt] at hm
      simp only [Option.some.injEq] at hm
      rw [hm', ← hm]
    subst this
    left
    exact ⟨k, hk, m', hm, hr, hpre, by simp⟩

/-- Where `remove_next` takes the run: the next doomed candidate, or the
end. -/
def RemoveNext (p : gc.GcState × gc.Command) : Prop :=
  (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
  (∃ (k : Usize) (c : retention.Candidate), s.doomed.val[s.index.val]? = some k ∧ s.candidates.val[k.val]? = some c ∧
     p = ({ s with phase := .Removing }, .Remove k c.stamp)) ∨
  (s.doomed.val.length ≤ s.index.val ∧ p = ({ s with phase := .Finished }, .Finish .Collected))

theorem remove_next_spec : gc.remove_next s ⦃ p => RemoveNext s p ⦄ := by
  unfold gc.remove_next RemoveNext
  dsimp only
  split_ifs with h
  · have hlt : s.index.val < s.doomed.val.length := by scalar_tac
    step as ⟨ k, hk ⟩
    split_ifs with hc
    · have hclt : k.val < s.candidates.val.length := by scalar_tac
      step as ⟨ c, hcv ⟩
      right; left
      refine ⟨k, c, ?_, ?_, rfl⟩
      · rw [List.getElem?_eq_getElem hlt]; simp [hk]
      · rw [List.getElem?_eq_getElem hclt]; simp [hcv]
    · simp [gc.refuse]
  · simp only [WP.spec_ok]
    right; right
    exact ⟨by scalar_tac, by simp⟩

end helpers

/-! ## The invariant of every run -/

/-- The inventory the machine admitted, and the fates it decided from it. -/
def AdmittedFacts (s : gc.GcState) : Prop :=
  s.inventory.recovery_pending = false ∧
  (∀ c, s.inventory.current = some c → c ∈ gens s) ∧
  s.prune.val = PrunePlan (gens s) s.inventory.current s.keep ∧
  ∀ m ∈ s.manifests.val, m.fate = .Retired → m.facts.id ∈ s.prune.val

/-- Every inventoried generation's manifest was read, newest first, and the
decided fates close. -/
def ReadFacts (s : gc.GcState) : Prop :=
  AdmittedFacts s ∧
  s.manifests.val.map (fun m => m.facts.id) = (gens s).reverse ∧
  ∀ m ∈ s.manifests.val, Closes s.manifests.val m (gens s)

/-- The deletion set is decided. -/
def PlannedFacts (s : gc.GcState) : Prop :=
  ReadFacts s ∧
  ∀ d : Usize, d ∈ s.doomed.val ↔ Unreferenced s.manifests.val s.external.val s.candidates.val d.val

/-- Every retired manifest's generation was retired in `cs`. -/
def AllRetired (ms : List retention.Manifest) (cs : List gc.Command) : Prop :=
  ∀ m ∈ ms, m.fate = .Retired → ∃ cur, gc.Command.Retire m.facts.id cur ∈ cs

theorem AllRetired.mono {ms cs} (h : AllRetired ms cs) (c : gc.Command) :
    AllRetired ms (cs ++ [c]) := fun m hm hr => by
  obtain ⟨cur, h⟩ := h m hm hr
  exact ⟨cur, List.mem_append_left _ h⟩

/-- What holds in each phase. -/
def Inv (s : gc.GcState) (cs : List gc.Command) : Prop :=
  match s.phase with
  | .Start | .ReadingInventory => s.manifests.val = []
  | .ReadingManifests => AdmittedFacts s ∧ s.index.val ≤ (gens s).length ∧
      s.manifests.val.map (fun m => m.facts.id) = ((gens s).drop s.index.val).reverse
  | .ReadingRoots | .ReadingCandidates => ReadFacts s
  | .Planning | .Finished => PlannedFacts s
  | .SealingFloor => PlannedFacts s ∧ s.mode = .Collect
  | .Retiring => PlannedFacts s ∧ s.mode = .Collect ∧ s.index.val < s.manifests.val.length ∧
      ∀ j (h : j < s.manifests.val.length), j ≤ s.index.val → s.manifests.val[j].fate = .Retired →
        ∃ cur, gc.Command.Retire s.manifests.val[j].facts.id cur ∈ cs
  | .Sealing => PlannedFacts s ∧ s.mode = .Collect ∧ AllRetired s.manifests.val cs ∧
      gc.Command.Barrier ∈ cs
  | .Delegating | .Removing => PlannedFacts s ∧ s.mode = .Collect ∧
      AllRetired s.manifests.val cs ∧ gc.Command.Barrier ∈ cs ∧ gc.Command.Delegate ∈ cs
  | .Aborted => True

/-- Phases with an effect outstanding. -/
def Effecting (p : gc.Phase) : Prop :=
  p = .SealingFloor ∨ p = .Retiring ∨ p = .Sealing ∨ p = .Delegating ∨ p = .Removing

/-- A command that changes the home. -/
def Effect (c : gc.Command) : Prop :=
  c = .SealFloor ∨ (∃ g cur, c = .Retire g cur) ∨ c = .Barrier ∨ c = .Delegate ∨
  (∃ k x, c = .Remove k x)

/-- At an effect, the deletion set is decided and the run collects. -/
theorem Inv.effecting {s cs} (h : Inv s cs) (he : Effecting s.phase) :
    PlannedFacts s ∧ s.mode = .Collect := by
  unfold Inv at h
  rcases he with hp | hp | hp | hp | hp <;> rw [hp] at h <;> simp only at h
  · exact h
  · exact ⟨h.1, h.2.1⟩
  · exact ⟨h.1, h.2.1⟩
  · exact ⟨h.1, h.2.1⟩
  · exact ⟨h.1, h.2.1⟩

/-- What one step guarantees, given the invariant before it. -/
def StepPost (s : gc.GcState) (cs : List gc.Command) (p : gc.GcState × gc.Command) : Prop :=
  Inv p.1 (cs ++ [p.2]) ∧ p.1.mode = s.mode ∧
  (Effect p.2 → Effecting p.1.phase) ∧
  (∀ g cur, p.2 = .Retire g cur →
    cur = p.1.inventory.current ∧ (∃ m ∈ p.1.manifests.val, m.facts.id = g ∧ m.fate = .Retired) ∧
    ∃ (k : Nat) (m : retention.Manifest), p.1.manifests.val[k]? = some m ∧ m.facts.id = g ∧
      ∀ j (h : j < p.1.manifests.val.length), j < k → p.1.manifests.val[j].fate = .Retired →
        ∃ cur', gc.Command.Retire p.1.manifests.val[j].facts.id cur' ∈ cs) ∧
  (∀ k x, p.2 = .Remove k x → p.1.phase = .Removing ∧
    ∃ c, p.1.candidates.val[k.val]? = some c ∧ x = c.stamp ∧ k ∈ p.1.doomed.val)

theorem post_abort {s : gc.GcState} {cs} (t : gc.GcState) (r : gc.Refusal) (hm : t.mode = s.mode) :
    StepPost s cs ({ t with phase := .Aborted }, .Abort r) := by
  refine ⟨by simp [Inv], hm, ?_, by simp, by simp⟩
  rintro (h | ⟨_, _, h⟩ | h | h | ⟨_, _, h⟩) <;> simp at h

/-- A step to a phase without an effect, a retirement or a removal. -/
theorem post_quiet {s : gc.GcState} {cs} {p : gc.GcState × gc.Command}
    (hi : Inv p.1 (cs ++ [p.2])) (hm : p.1.mode = s.mode)
    (he : ¬ Effect p.2) (hr : ∀ g cur, p.2 ≠ .Retire g cur) (hx : ∀ k x, p.2 ≠ .Remove k x) :
    StepPost s cs p :=
  ⟨hi, hm, fun h => absurd h he, fun g cur h => absurd h (hr g cur), fun k x h => absurd h (hx k x)⟩

theorem drop_pred_reverse {l : List generation.GenerationId} {k : Nat} {g}
    (hk : 0 < k) (hg : l[k - 1]? = some g) :
    (l.drop (k - 1)).reverse = (l.drop k).reverse ++ [g] := by
  obtain ⟨hlt, hget⟩ := List.getElem?_eq_some_iff.mp hg
  rw [List.drop_eq_getElem_cons hlt, List.reverse_cons, hget]
  congr 3
  omega

/-- Entering `next_manifest` from a state whose facts are admitted. -/
theorem post_next_manifest {s : gc.GcState} {cs} {t : gc.GcState} {p}
    (hp : NextManifest t p) (ha : AdmittedFacts t) (hi : t.index.val ≤ (gens t).length)
    (hr : t.manifests.val.map (fun m => m.facts.id) = ((gens t).drop t.index.val).reverse)
    (hm : t.mode = s.mode) :
    StepPost s cs p := by
  rcases hp with ⟨r, rfl⟩ | ⟨_, g, _, rfl⟩ | ⟨h0, hc, rfl⟩
  · exact post_abort t r hm
  · exact post_quiet (by simp only [Inv]; exact ⟨ha, hi, hr⟩) hm (by simp [Effect]) (by simp)
      (by simp)
  · refine post_quiet ?_ hm (by simp [Effect]) (by simp) (by simp)
    simp only [Inv]
    refine ⟨ha, ?_, hc⟩
    rw [hr, h0, List.drop_zero]

/-- Leaving a decided state through `retire_next`. -/
theorem post_retire_next {s t : gc.GcState} {cs} {p} (hp : RetireNext t p)
    (hpl : PlannedFacts t) (hc : t.mode = .Collect) (hm : t.mode = s.mode)
    (hprev : ∀ j (h : j < t.manifests.val.length), j < t.index.val →
      t.manifests.val[j].fate = .Retired → ∃ cur, gc.Command.Retire t.manifests.val[j].facts.id cur ∈ cs) :
    StepPost s cs p := by
  rcases hp with ⟨k, hk, m, hm', hret, hpre, rfl⟩ | ⟨hall, rfl⟩
  · have hlt : k.val < t.manifests.val.length := (List.getElem?_eq_some_iff.mp hm').1
    have hmk : t.manifests.val[k.val] = m := (List.getElem?_eq_some_iff.mp hm').2
    refine ⟨?_, hm, fun _ => by simp [Effecting], ?_, by simp⟩
    · simp only [Inv]
      refine ⟨hpl, hc, hlt, fun j hj hjk hr => ?_⟩
      rcases Nat.lt_or_ge j t.index.val with hlt' | hge
      · obtain ⟨cur, h⟩ := hprev j hj hlt' hr
        exact ⟨cur, List.mem_append_left _ h⟩
      · rcases Nat.lt_or_eq_of_le hjk with hlt'' | heq
        · rw [hpre j hj hge hlt''] at hr; cases hr
        · subst heq
          exact ⟨t.inventory.current, by simp [hmk]⟩
    · intro g cur h
      simp only [gc.Command.Retire.injEq] at h
      obtain ⟨rfl, rfl⟩ := h
      refine ⟨rfl, ⟨m, List.mem_of_getElem? hm', rfl, hret⟩, k.val, m, hm', rfl,
        fun j hj hjk hr => ?_⟩
      rcases Nat.lt_or_ge j t.index.val with hlt' | hge
      · exact hprev j hj hlt' hr
      · rw [hpre j hj hge hjk] at hr; cases hr
  · refine ⟨?_, hm, fun _ => by simp [Effecting], by simp, by simp⟩
    simp only [Inv]
    refine ⟨hpl, hc, fun m hmem hr => ?_, by simp⟩
    obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hmem
    rcases Nat.lt_or_ge j t.index.val with hlt' | hge
    · obtain ⟨cur, h⟩ := hprev j hj hlt' hr
      exact ⟨cur, List.mem_append_left _ h⟩
    · rw [hall j hj hge] at hr; cases hr

/-- Leaving a decided state through `remove_next`. -/
theorem post_remove_next {s t : gc.GcState} {cs} {p} (hp : RemoveNext t p)
    (hpl : PlannedFacts t) (hc : t.mode = .Collect) (hm : t.mode = s.mode)
    (hr : AllRetired t.manifests.val cs)
    (hb : gc.Command.Barrier ∈ cs) (hd : gc.Command.Delegate ∈ cs) :
    StepPost s cs p := by
  rcases hp with ⟨r, rfl⟩ | ⟨k, c, hk, hcand, rfl⟩ | ⟨_, rfl⟩
  · exact post_abort t r hm
  · refine ⟨?_, hm, fun _ => by simp [Effecting], by simp, ?_⟩
    · simp only [Inv]
      exact ⟨hpl, hc, hr.mono _, List.mem_append_left _ hb, List.mem_append_left _ hd⟩
    · intro k' x h
      simp only [gc.Command.Remove.injEq] at h
      obtain ⟨rfl, rfl⟩ := h
      exact ⟨rfl, c, hcand, rfl, List.mem_of_getElem? hk⟩
  · exact post_quiet (by simp only [Inv]; exact hpl) hm (by simp [Effect]) (by simp) (by simp)

theorem step_spec (s : gc.GcState) (e : gc.Event) (cs : List gc.Command) (h : Inv s cs) :
    gc.gc_step s e ⦃ p => StepPost s cs p ⦄ := by
  rcases s with ⟨phase, mode, keep, inventory, prune, manifests, external, candidates, doomed, index⟩
  unfold gc.gc_step
  cases phase <;> cases e
  all_goals try first
    | (with_reducible exact WP.spec_mono (failed_or_protocol_spec' _ _) (by rintro p ⟨r, rfl⟩; exact post_abort _ r rfl))
    | (with_reducible exact WP.spec_mono (refuse_spec' _ _) (by rintro p rfl; exact post_abort _ _ rfl))
  case Start.Start =>
    simp only [WP.spec_ok]
    exact post_quiet (by simpa [Inv] using h) rfl (by simp [Effect]) (by simp) (by simp)
  case ReadingInventory.Inventory f =>
    simp only [Inv] at h
    refine WP.spec_mono (on_inventory_spec _ f) ?_
    rintro p (⟨r, rfl⟩ | ⟨hp, hc, v, i, hv, hi, hn⟩)
    · exact post_abort _ r rfl
    · refine post_next_manifest hn ⟨hp, hc, hv, by simp [h]⟩ (by simp [hi]) ?_ rfl
      simp [h, hi]
  case ReadingManifests.Manifest f =>
    simp only [Inv] at h
    obtain ⟨ha, hi, hr⟩ := h
    simp only [gens] at hi hr
    have hlen : manifests.val.length + index.val = (inventory.generations.ids.val).length := by
      have := congrArg List.length hr
      simp only [List.length_map, List.length_reverse, List.length_drop] at this
      omega
    refine WP.spec_mono (on_manifest_spec _ f hlen) ?_
    rintro p (⟨r, rfl⟩ | ⟨hpos, hg, fate, v, i, hfate, hv, hi', hn⟩)
    · exact post_abort _ r rfl
    · obtain ⟨hp, hc, hpr, hf⟩ := ha
      simp only at hi' hpos hg hfate
      refine post_next_manifest hn ⟨hp, hc, hpr, ?_⟩ (by simp only [gens] at hi ⊢; omega) ?_ rfl
      · intro m hm hret
        simp only [hv, List.mem_append, List.mem_singleton] at hm
        rcases hm with hm | rfl
        · exact hf m hm hret
        · exact hfate hret
      · simp only [hv, hi', List.map_append, List.map_cons, List.map_nil]
        rw [hr]
        exact (drop_pred_reverse hpos hg).symm
  case ReadingRoots.Roots ext =>
    simp only [WP.spec_ok]
    exact post_quiet (by simpa [Inv, ReadFacts, AdmittedFacts] using h) rfl (by simp [Effect])
      (by simp) (by simp)
  case ReadingCandidates.Candidates cands =>
    simp only [Inv] at h
    refine WP.spec_mono (on_candidates_spec _ cands) ?_
    rintro p ⟨v, hv, rfl⟩
    exact post_quiet (by simp only [Inv]; exact ⟨h, hv⟩) rfl (by simp [Effect]) (by simp) (by simp)
  case Planning.Done =>
    simp only [Inv] at h
    cases mode
    · simp only [WP.spec_ok]
      exact post_quiet (by simp only [Inv]; exact h) rfl (by simp [Effect]) (by simp) (by simp)
    · simp only [WP.spec_ok]
      refine ⟨by simp only [Inv]; exact ⟨h, by simp⟩, rfl, fun _ => by simp [Effecting], by simp, by simp⟩
  case SealingFloor.Done =>
    simp only [Inv] at h
    exact WP.spec_mono (retire_next_spec _ (by simp))
      (fun p hp => post_retire_next hp h.1 h.2 rfl (fun j _ hj => absurd hj (by simp)))
  case Retiring.Done =>
    simp only [Inv] at h
    obtain ⟨hpl, hc, hlt, hprev⟩ := h
    have hadd := Usize.checked_add_bv_spec index 1#usize
    simp only [lift]
    cases ho : Usize.checked_add index 1#usize with
    | none => simp [gc.refuse]; exact post_abort _ _ rfl
    | some i =>
      simp only [ho] at hadd
      simp only [bind_ok]
      refine WP.spec_mono (retire_next_spec _ (by simp only; scalar_tac)) ?_
      intro p hp
      exact post_retire_next hp hpl hc rfl
        (fun j hj hji hr => hprev j hj (by simp only at hji hlt ⊢; scalar_tac) hr)
  case Sealing.Done =>
    simp only [Inv] at h
    obtain ⟨hpl, hc, hr, hb⟩ := h
    simp only [WP.spec_ok]
    refine ⟨?_, rfl, fun _ => by simp [Effecting], by simp, by simp⟩
    simp only [Inv]
    exact ⟨hpl, hc, hr.mono _, List.mem_append_left _ hb, by simp⟩
  case Delegating.Done =>
    simp only [Inv] at h
    obtain ⟨hpl, hc, hr, hb, hd⟩ := h
    exact WP.spec_mono (remove_next_spec _) (fun p hp => post_remove_next hp hpl hc rfl hr hb hd)
  case Removing.Done =>
    simp only [Inv] at h
    obtain ⟨hpl, hc, hr, hb, hd⟩ := h
    simp only [lift]
    cases ho : Usize.checked_add index 1#usize with
    | none => simp [gc.refuse]; exact post_abort _ _ rfl
    | some i =>
      simp only [bind_ok]
      exact WP.spec_mono (remove_next_spec _) (fun p hp => post_remove_next hp hpl hc rfl hr hb hd)

/-- The executions of the machine: from `GcState::new` in either mode, any
sequence of events the step accepts, with the commands it issued, in order. -/
inductive Run : gc.GcState → List gc.Command → Prop where
  | start {s mode keep} : gc.GcState.new mode keep = ok s → Run s []
  | step {s cs e s' c} : Run s cs → gc.gc_step s e = ok (s', c) → Run s' (cs ++ [c])

theorem run_inv {s : gc.GcState} {cs : List gc.Command} (h : Run s cs) : Inv s cs := by
  induction h with
  | start hs =>
    simp only [gc.GcState.new, generation.GenerationList.empty, bind_ok, ok.injEq] at hs
    subst hs
    simp [Inv]
  | step _ hstep ih => exact (spec_eq_ok (step_spec _ _ _ ih) hstep).1

theorem run_post {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event} {c : gc.Command}
    (hrun : Run s cs) (h : gc.gc_step s e = ok (s', c)) : StepPost s cs (s', c) :=
  spec_eq_ok (step_spec _ _ _ (run_inv hrun)) h

theorem run_mode {s : gc.GcState} {cs : List gc.Command} (h : Run s cs) :
    ∃ mode keep s₀, gc.GcState.new mode keep = ok s₀ ∧ s.mode = mode := by
  induction h with
  | @start s mode keep hs =>
    refine ⟨mode, keep, s, hs, ?_⟩
    simp only [gc.GcState.new, generation.GenerationList.empty, bind_ok, ok.injEq] at hs
    rw [← hs]
  | step hrun hstep ih =>
    obtain ⟨mode, keep, s₀, h₀, hm⟩ := ih
    exact ⟨mode, keep, s₀, h₀, (run_post hrun hstep).2.1.trans hm⟩

/-! ## The theorems -/

/-- (K4) Nothing is deleted while recovery or activation is pending, nor in a
preview: in every execution, an effect is issued only in collect mode, from an
admitted inventory — no recovery or activation pending, and `current` (if
any) among the inventoried generations. -/
theorem gc_effects_admitted {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {c : gc.Command} (hrun : Run s cs) (h : gc.gc_step s e = ok (s', c)) (he : Effect c) :
    s'.mode = .Collect ∧ s'.inventory.recovery_pending = false ∧
      ∀ cur, s'.inventory.current = some cur → cur ∈ s'.inventory.generations.ids.val := by
  have hp := run_post hrun h
  obtain ⟨⟨⟨⟨hpend, hcur, -, -⟩, -, -⟩, -⟩, hc⟩ := hp.1.effecting (hp.2.2.1 he)
  exact ⟨hc, hpend, hcur⟩

/-- A preview changes nothing: in every execution of a machine whose mode is
`Preview`, no effect is ever issued. -/
theorem gc_preview_effect_free {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {c : gc.Command} (hrun : Run s cs) (h : gc.gc_step s e = ok (s', c))
    (hpre : s.mode = .Preview) : ¬ Effect c := by
  intro he
  have hc := (gc_effects_admitted hrun h he).1
  rw [(run_post hrun h).2.1, hpre] at hc
  cases hc

/-- (K4) A retired generation is in the prune plan of the admitted inventory —
inventoried, never `current` — and no retained manifest names it as a
predecessor. The command carries the inventoried `current`, which the driver
re-checks before the rename. -/
theorem gc_retire_pruned {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {g : generation.GenerationId} {cur : Option generation.GenerationId}
    (hrun : Run s cs) (h : gc.gc_step s e = ok (s', .Retire g cur)) :
    cur = s'.inventory.current ∧
      g ∈ PrunePlan s'.inventory.generations.ids.val s'.inventory.current s'.keep ∧
      ¬ ∃ m ∈ s'.manifests.val, Needs m g := by
  have hp := run_post hrun h
  obtain ⟨hcur, ⟨m, hm, rfl, hret⟩, -⟩ := hp.2.2.2.1 g cur rfl
  obtain ⟨⟨⟨⟨-, -, hprune, hfates⟩, -, hclosed⟩, -⟩, -⟩ :=
    hp.1.effecting (hp.2.2.1 (Or.inr (Or.inl ⟨_, _, rfl⟩)))
  refine ⟨hcur, ?_, (hclosed m hm).2 hret⟩
  rw [← hprune]
  exact hfates m hm hret

/-- (K4) The current generation is never retired, and only inventoried
generations are. -/
theorem gc_never_retires_current {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {g : generation.GenerationId} {cur : Option generation.GenerationId}
    (hrun : Run s cs) (h : gc.gc_step s e = ok (s', .Retire g cur)) :
    s'.inventory.current ≠ some g ∧ g ∈ s'.inventory.generations.ids.val := by
  obtain ⟨-, hplan, -⟩ := gc_retire_pruned hrun h
  exact (PrunePlan.mem hplan).symm

/-- (K4) Retirement is newest first. When a generation is retired, every
retired generation before it in the manifest order — the inventory in
reverse, newest first (`gc_retained_closed`) — was already retired. Receipt
predecessors are strictly older, so an interrupted batch never leaves a live
receipt whose predecessor is gone. -/
theorem gc_retire_newest_first {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {g : generation.GenerationId} {cur : Option generation.GenerationId}
    (hrun : Run s cs) (h : gc.gc_step s e = ok (s', .Retire g cur)) :
    s'.manifests.val.map (fun m => m.facts.id) = s'.inventory.generations.ids.val.reverse ∧
    ∃ (k : Nat) (m : retention.Manifest), s'.manifests.val[k]? = some m ∧ m.facts.id = g ∧
      ∀ j (h : j < s'.manifests.val.length), j < k → s'.manifests.val[j].fate = .Retired →
        ∃ cur', gc.Command.Retire s'.manifests.val[j].facts.id cur' ∈ cs := by
  have hp := run_post hrun h
  obtain ⟨-, -, horder⟩ := hp.2.2.2.1 g cur rfl
  obtain ⟨⟨⟨-, hcov, -⟩, -⟩, -⟩ := hp.1.effecting (hp.2.2.1 (Or.inr (Or.inl ⟨_, _, rfl⟩)))
  exact ⟨hcov, horder⟩

/-- (K4) deletions ⊆ candidates − roots: a removal names a candidate the host
inventoried, carries its inventoried stamp as the driver's re-check, and no
retained manifest pins it and no external root names it. -/
theorem gc_remove_unreferenced {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {k : Usize} {x : retention.Stamp}
    (hrun : Run s cs) (h : gc.gc_step s e = ok (s', .Remove k x)) :
    ∃ c, s'.candidates.val[k.val]? = some c ∧ x = c.stamp ∧
      ¬ Rooted s'.manifests.val s'.external.val c.path.val := by
  have hp := run_post hrun h
  obtain ⟨-, c, hc, hx, hk⟩ := hp.2.2.2.2 k x rfl
  obtain ⟨⟨-, hdoomed⟩, -⟩ := hp.1.effecting (hp.2.2.1 (Or.inr (Or.inr (Or.inr (Or.inr ⟨_, _, rfl⟩)))))
  obtain ⟨c', hc', hroot⟩ := (hdoomed k).mp hk
  rw [hc] at hc'
  cases hc'
  exact ⟨c, hc, hx, hroot⟩

/-- The deletion set is exactly the unreferenced candidates: whenever an
effect is issued, nothing a root names is in it, and nothing unreferenced is
left out. -/
theorem gc_doomed_exact {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {c : gc.Command} (hrun : Run s cs) (h : gc.gc_step s e = ok (s', c)) (he : Effect c) :
    ∀ d : Usize, d ∈ s'.doomed.val ↔
      Unreferenced s'.manifests.val s'.external.val s'.candidates.val d.val := by
  have hp := run_post hrun h
  exact (hp.1.effecting (hp.2.2.1 he)).1.2

/-- (K4′) The roots are the closure of every retained manifest: whenever an
effect is issued, every inventoried generation's manifest was read, every
retained generation's predecessors are inventoried, and a manifest of such a
predecessor is retained. -/
theorem gc_retained_closed {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {c : gc.Command} (hrun : Run s cs) (h : gc.gc_step s e = ok (s', c)) (he : Effect c) :
    (∀ g ∈ s'.inventory.generations.ids.val, ∃ m ∈ s'.manifests.val, m.facts.id = g) ∧
    ∀ m ∈ s'.manifests.val, m.fate = .Retained → ∀ p ∈ m.facts.predecessors.val,
      p ∈ s'.inventory.generations.ids.val ∧
      ∀ m' ∈ s'.manifests.val, m'.facts.id = p → m'.fate = .Retained := by
  have hp := run_post hrun h
  obtain ⟨⟨⟨-, hcov, hclosed⟩, -⟩, -⟩ := hp.1.effecting (hp.2.2.1 he)
  refine ⟨fun g hg => ?_, fun m hm hret p hpred => ⟨(hclosed m hm).1 hret p hpred, ?_⟩⟩
  · have : g ∈ s'.manifests.val.map (fun m => m.facts.id) := by
      rw [hcov]; exact List.mem_reverse.mpr hg
    simpa using this
  · intro m' hm' hid
    cases hf : m'.fate
    · rfl
    · exact absurd ⟨m, hm, hret, hid ▸ hpred⟩ ((hclosed m' hm').2 hf)

/-- Every removal follows the retirement of every retired generation, the
barrier on `generations/` that makes those retirements durable, and the
delegated retirement of their receipts: a payload goes only once no
surviving generation can name it, even after a power loss. -/
theorem gc_remove_after_retirement {s s' : gc.GcState} {cs : List gc.Command} {e : gc.Event}
    {k : Usize} {x : retention.Stamp}
    (hrun : Run s cs) (h : gc.gc_step s e = ok (s', .Remove k x)) :
    AllRetired s'.manifests.val cs ∧ gc.Command.Barrier ∈ cs ∧ gc.Command.Delegate ∈ cs := by
  have hp := run_post hrun h
  obtain ⟨hph, -⟩ := hp.2.2.2.2 k x rfl
  have hi := hp.1
  unfold Inv at hi
  rw [hph] at hi
  obtain ⟨-, -, hr, hb, hd⟩ := hi
  refine ⟨fun m hm hret => ?_, ?_, ?_⟩
  · obtain ⟨cur, hmem⟩ := hr m hm hret
    rcases List.mem_append.mp hmem with h' | h'
    · exact ⟨cur, h'⟩
    · simp at h'
  · rcases List.mem_append.mp hb with h' | h'
    · exact h'
    · simp at h'
  · rcases List.mem_append.mp hd with h' | h'
    · exact h'
    · simp at h'

end Gripsack.Kernel.Gc
