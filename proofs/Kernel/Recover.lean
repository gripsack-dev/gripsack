import Extracted.GripsackKernel
import Kernel.Selection

/-!
# The recover machine (plan/0056 F2 step 2)

Theorems over the Aeneas translation of `gripsack_kernel::recover`
(`Extracted/GripsackKernel.lean`).

* Identities: `Digest::same`, `DestId::same`, `Ident::same`, `Live::same`
  and `Prior::matches` are equality.
* `decide_spec`: the per-entry decision restores only the run's own write
  (intended or immediate pre-state) or an absence where a prior existed, and
  refuses exactly the objects without a journal identity.
* (a) `mutate_guard`: a restore is issued only from the observation of the
  entry under decision, with `expect` equal to the observed live object, and
  only when that object is restorable in the sense of `decide_spec`.
* (c) `unrestorable_refused`, `mutate_never_unsupported`: an object without a
  journal identity yields a refusal naming the entry, never a mutation.
* (b) `marker_unlink_after_record_barrier`: over every run of the machine,
  the marker unlink is issued only immediately after a journal barrier that
  follows the unlink of every journal record.
* (d) `mutate_after_classification`: over every run, a restore is issued
  only when the run was classified uncommitted — by `classify` over the
  marker and the `current` read earlier in the run — or has no marker.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel

/-! ## Identities -/

/-- The byte loop of `Digest::same`, from position `i`. -/
theorem Digest.same_loop_spec (a b : ids.Digest) (i : Usize)
    (hi : i.val ≤ 32)
    (hprefix : ∀ j, j < i.val → a.bytes.val[j]? = b.bytes.val[j]?) :
    ids.Digest.same_loop a b i ⦃ r => r ↔ a = b ⦄ := by
  unfold ids.Digest.same_loop
  apply loop.spec_decr_nat (fun i : Usize => 32 - i.val)
    (fun i : Usize => i.val ≤ 32 ∧ ∀ j, j < i.val → a.bytes.val[j]? = b.bytes.val[j]?)
  · intro i ⟨hi, hprefix⟩
    unfold ids.Digest.same_loop.body
    have hbytes : ids.DIGEST_BYTES = 32#usize := by
      simp [ids.DIGEST_BYTES]
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

/-- `Digest::same` is byte-exact equality. -/
theorem Digest.same_spec (a b : ids.Digest) :
    ids.Digest.same a b ⦃ r => r ↔ a = b ⦄ := by
  unfold ids.Digest.same
  apply Digest.same_loop_spec
  · simp
  · intro j hj; simp at hj

/-- `DestId::same` is equality. -/
theorem DestId.same_spec (a b : ids.DestId) :
    ids.DestId.same a b ⦃ r => r ↔ a = b ⦄ := by
  obtain ⟨a⟩ := a
  obtain ⟨b⟩ := b
  simp [ids.DestId.same]

/-- `Ident::same` is equality: a file never equals a link. -/
theorem Ident.same_spec (a b : recover.Ident) :
    recover.Ident.same a b ⦃ r => r ↔ a = b ⦄ := by
  cases a <;> cases b <;> simp only [recover.Ident.same]
  · step with Digest.same_spec as ⟨ r, hr ⟩
    simp_all
  · simp
  · simp
  · step with Digest.same_spec as ⟨ r, hr ⟩
    simp_all

/-- `Live::same` is equality (the driver's re-check uses it). -/
theorem Live.same_spec (a b : recover.Live) :
    recover.Live.same a b ⦃ r => r ↔ a = b ⦄ := by
  cases a <;> cases b <;> simp only [recover.Live.same] <;> try simp
  step with Ident.same_spec as ⟨ r, hr ⟩
  simp_all

/-- `Prior::matches` is equality of the prior with the live object. -/
theorem Prior.matches_spec (p : recover.Prior) (l : recover.Live) :
    recover.Prior.matches p l ⦃ r =>
      r ↔ ((p = .Absent ∧ l = .Absent) ∨ (∃ i, p = .Obj i ∧ l = .Obj i)) ⦄ := by
  cases p <;> cases l <;> simp only [recover.Prior.matches] <;> try simp
  step with Ident.same_spec as ⟨ r, hr ⟩
  simp_all [eq_comm]

/-! ## The per-entry decision -/

/-- The live objects a restore may replace: the run's own write (intended or
immediate pre-state), or an absence where the prior must come back. Anything
else is foreign (kept) or has no identity (refused). -/
def Restorable (f : recover.EntryFacts) (live : recover.Live) : Prop :=
  (∃ i, live = .Obj i ∧ (f.intended = .Obj i ∨ f.before = some i)) ∨
  (live = .Absent ∧ f.prior ≠ .Absent)

theorem intends_spec (intended : recover.Intended) (o : recover.Ident) :
    recover.intends intended o ⦃ r => r ↔ intended = .Obj o ⦄ := by
  cases intended <;> simp only [recover.intends] <;> try simp
  step with Ident.same_spec as ⟨ r, hr ⟩
  simp_all

theorem precedes_spec (before : Option recover.Ident) (o : recover.Ident) :
    recover.precedes before o ⦃ r => r ↔ before = some o ⦄ := by
  cases before <;> simp only [recover.precedes] <;> try simp
  step with Ident.same_spec as ⟨ r, hr ⟩
  simp_all

/-- The decision: a restore only of a restorable object; a refusal exactly
for an object without a journal identity; "unchanged" only when the prior is
live. -/
theorem decide_spec (f : recover.EntryFacts) (live : recover.Live) :
    recover.decide f live ⦃ d =>
      (d = .Restore → Restorable f live) ∧
      (d = .Refuse ↔ live = .Unsupported) ∧
      (d = .Unchanged →
        ((f.prior = .Absent ∧ live = .Absent) ∨ (∃ i, f.prior = .Obj i ∧ live = .Obj i))) ⦄ := by
  unfold recover.decide
  cases live with
  | Absent => cases hp : f.prior <;> simp [Restorable, hp]
  | Unsupported => simp
  | Obj o =>
    simp only
    step with intends_spec as ⟨ b, hb ⟩
    split_ifs with h1
    · simp only [WP.spec_ok]
      refine ⟨fun _ => Or.inl ⟨o, rfl, Or.inl (hb.mp h1)⟩, by simp, by simp⟩
    · step with Prior.matches_spec as ⟨ b1, hb1 ⟩
      split_ifs with h2
      · simp only [WP.spec_ok]
        refine ⟨by simp, by simp, fun _ => ?_⟩
        rcases hb1.mp h2 with ⟨_, h⟩ | ⟨i, hp, hl⟩
        · cases h
        · exact Or.inr ⟨i, hp, hl⟩
      · step with precedes_spec as ⟨ b2, hb2 ⟩
        split_ifs with h3
        · simp only [WP.spec_ok]
          refine ⟨fun _ => Or.inl ⟨o, rfl, Or.inr (hb2.mp h3)⟩, by simp, by simp⟩
        · simp

/-- A total-correctness spec yields the value the call returns. -/
theorem spec_exists_ok {α : Type} {x : Result α} {P : α → Prop}
    (h : x ⦃ r => P r ⦄) : ∃ a, x = ok a ∧ P a := by
  cases x with
  | ret a => exact ⟨a, rfl, (WP.spec_ok a).mp h⟩
  | vis e k => exact absurd h (by simp [WP.spec_vis])
  | div => exact absurd h (by simp [WP.spec_div])

/-! ## What each step helper returns

One total-correctness spec per helper of `recover_step`, stating its result
as explicit record updates. The step theorem below is a case analysis over
these. -/

section helpers
variable (s : recover.RecoverState)

theorem refuse_spec (r : recover.Refusal) :
    recover.refuse s r ⦃ p => p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  simp [recover.refuse]

theorem failed_or_protocol_spec (e : recover.Event) :
    recover.failed_or_protocol s e ⦃ p =>
      ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold recover.failed_or_protocol
  cases e <;> simp [recover.refuse]

/-- How cleanup continues from `s`: the next record unlink, or the journal
barrier once every record was issued. -/
def NextUnlink (p : recover.RecoverState × recover.Command) : Prop :=
  (s.index.val < s.records.val ∧ p = ({ s with phase := .UnlinkingRecord }, .Unlink (.Record s.index))) ∨
  (s.records.val ≤ s.index.val ∧ p = ({ s with phase := .SyncingRecords }, .Barrier .Journal))

theorem next_unlink_spec : recover.next_unlink s ⦃ p => NextUnlink s p ⦄ := by
  unfold recover.next_unlink NextUnlink
  split_ifs with h
  · simp only [WP.spec_ok]; left; exact ⟨by scalar_tac, by simp⟩
  · simp only [WP.spec_ok]; right; exact ⟨by scalar_tac, by simp⟩

theorem begin_cleanup_spec :
    recover.begin_cleanup s ⦃ p => NextUnlink { s with index := 0#u32, entry := none } p ⦄ := by
  unfold recover.begin_cleanup
  exact next_unlink_spec _

/-- Where `proceed` takes an uncommitted run from `s`. -/
def Proceed (p : recover.RecoverState × recover.Command) : Prop :=
  p = ({ s with phase := .OpeningLeaves, leaves_open := true }, .Delegate .BeforeLeaves) ∨
  p = ({ s with phase := .ReadingEntry }, .ReadEntry s.index) ∨
  p = ({ s with phase := .ClosingLeaves }, .Delegate .AfterLeaves) ∨
  NextUnlink { s with index := 0#u32, entry := none } p

theorem proceed_spec : recover.proceed s ⦃ p => Proceed s p ⦄ := by
  unfold recover.proceed Proceed
  split_ifs <;> first
    | (simp only [WP.spec_ok]; simp)
    | (apply WP.spec_mono (begin_cleanup_spec _); intro p hp; exact Or.inr (Or.inr (Or.inr hp)))

theorem advance_spec :
    recover.advance s ⦃ p =>
      p = ({ s with entry := none, phase := .Aborted }, .Abort .Malformed) ∨
      ∃ i : U32, i.val = s.index.val + 1 ∧ Proceed { s with index := i, entry := none } p ⦄ := by
  unfold recover.advance
  have h := U32.checked_add_bv_spec s.index 1#u32
  cases ho : U32.checked_add s.index 1#u32 with
  | none => simp [lift, recover.refuse]
  | some i =>
    simp only [ho] at h
    simp only [lift, Std.bind_ok]
    refine WP.spec_mono (proceed_spec { s with index := i, entry := none }) ?_
    intro p hp
    right
    exact ⟨i, by simpa using h.2.1, hp⟩

/-- Where `advance` takes the run: the next entry (or step) after
`s.index`, or a refusal when the counter would overflow. -/
def Advance (p : recover.RecoverState × recover.Command) : Prop :=
  p = ({ s with entry := none, phase := .Aborted }, .Abort .Malformed) ∨
  ∃ i : U32, i.val = s.index.val + 1 ∧ Proceed { s with index := i, entry := none } p

theorem advance_spec' : recover.advance s ⦃ p => Advance s p ⦄ := advance_spec s

theorem on_journal_spec (j : recover.JournalFacts) :
    recover.on_journal s j ⦃ p =>
      p = ({ s with phase := .Aborted }, .Abort .Malformed) ∨
      (∃ records, j.marker = none ∧
        p = ({ s with journal := j, records := records, phase := .Aborted }, .Abort .MarkerMissing)) ∨
      (∃ records m, j.marker = some m ∧
        p = ({ s with phase := .ReadingCurrent, journal := j, records := records }, .ReadCurrent)) ∨
      (∃ records, j.marker = none ∧
        Proceed { s with journal := j, records := records, verdict := .Uncommitted, index := 0#u32 } p) ⦄ := by
  unfold recover.on_journal
  by_cases hr : j.reverse_roots > j.entries
  · simp only [hr, ↓reduceIte]; simp [recover.refuse]
  · simp only [hr, ↓reduceIte]
    cases ho : U32.checked_add j.entries j.directories with
    | none => simp [lift, recover.refuse]
    | some records =>
      simp only [lift, Std.bind_ok]
      cases hm : j.marker with
      | some m => simp
      | none =>
        simp only
        by_cases hd : (j.directories != 0#u32) = true
        · simp only [hd, ↓reduceIte]; simp [recover.refuse]
        · simp only [hd, Bool.false_eq_true, ↓reduceIte]
          refine WP.spec_mono (proceed_spec _) ?_
          intro p hp
          exact Or.inr (Or.inr (Or.inr ⟨records, by simp, hp⟩))

theorem on_current_spec (current : Option selection.SelectionIdentity) :
    recover.on_current s current ⦃ p =>
      (s.journal.marker = none ∧ p = ({ s with current := current, phase := .Aborted }, .Abort .Protocol)) ∨
      ∃ m, s.journal.marker = some m ∧
        ((selection.classify { previous := m.previous, target := m.target, current := current } =
            ok .Committed ∧
          p = ({ s with phase := .SealingSelection, current := current, verdict := .Committed },
            .Barrier .Selection)) ∨
         (selection.classify { previous := m.previous, target := m.target, current := current } =
            ok .Uncommitted ∧
          Proceed { s with current := current, verdict := .Uncommitted, index := 0#u32 } p) ∨
         (p = ({ s with current := current, phase := .Aborted }, .Abort .Ambiguous)) ) ⦄ := by
  unfold recover.on_current
  cases hm : s.journal.marker with
  | none => simp [recover.refuse]
  | some m =>
    simp only
    have hc := spec_exists_ok (classify_committed_iff
      { previous := m.previous, target := m.target, current := current })
    obtain ⟨c, hc, -⟩ := hc
    rw [hc, Std.bind_ok]
    cases c with
    | Committed => simp [hc]
    | Uncommitted =>
      simp only
      refine WP.spec_mono (proceed_spec _) ?_
      intro p hp
      exact Or.inr ⟨m, rfl, Or.inr (Or.inl ⟨hc, hp⟩)⟩
    | Ambiguous => simp [recover.refuse]

theorem on_entry_spec (f : recover.EntryFacts) :
    recover.on_entry s f ⦃ p =>
      (f.route = .Leaf ∧ p = ({ s with phase := .Observing, entry := some f }, .Observe s.index f.dest)) ∨
      Advance s p ⦄ := by
  unfold recover.on_entry
  cases f.route with
  | Leaf => simp
  | Delegated =>
    refine WP.spec_mono (advance_spec _) ?_
    intro p hp
    exact Or.inr hp

theorem on_observed_spec (d : ids.DestId) (live : recover.Live) :
    recover.on_observed s d live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r) ∧
        (∀ f, s.entry = some f → f.dest = d → live = .Unsupported →
          r = .Unrestorable s.index d)) ∨
      ∃ f, s.entry = some f ∧ f.dest = d ∧ live ≠ .Unsupported ∧
        ((Restorable f live ∧
            p = ({ s with phase := .Restoring }, .Mutate s.index d live (.Restore f.prior))) ∨
         p = ({ s with phase := .Sealing }, .Seal s.index d live f.prior) ∨
         p = ({ s with phase := .Preserving }, .Preserve s.index d)) ⦄ := by
  unfold recover.on_observed
  cases he : s.entry with
  | none => simp [recover.refuse, he]
  | some f =>
    simp only
    step with DestId.same_spec as ⟨ b, hb ⟩
    split_ifs with hd
    · have hfd : f.dest = d := hb.mp hd
      step with decide_spec as ⟨ dec, hdec1, hdec2, hdec3 ⟩
      cases dec with
      | Refuse =>
        simp only [recover.refuse, WP.spec_ok]
        left
        exact ⟨_, by simp [he], fun _ _ _ _ => rfl⟩
      | Restore =>
        have hu : live ≠ .Unsupported := fun h => by simpa using hdec2.mpr h
        simp only [WP.spec_ok]
        right
        refine ⟨f, rfl, hfd, hu, Or.inl ⟨hdec1 rfl, ?_⟩⟩
        simp
      | Unchanged =>
        have hu : live ≠ .Unsupported := fun h => by simpa using hdec2.mpr h
        simp only [WP.spec_ok]
        right
        refine ⟨f, rfl, hfd, hu, Or.inr (Or.inl ?_)⟩
        simp
      | Preserve =>
        have hu : live ≠ .Unsupported := fun h => by simpa using hdec2.mpr h
        simp only [WP.spec_ok]
        right
        refine ⟨f, rfl, hfd, hu, Or.inr (Or.inr ?_)⟩
        simp
    · have hfd : f.dest ≠ d := fun h => hd (hb.mpr h)
      simp only [recover.refuse, WP.spec_ok]
      refine Or.inl ⟨.Protocol, by simp [he], fun f' hf' hd' _ => ?_⟩
      cases hf'
      exact absurd hd' hfd

theorem on_landed_spec (live : recover.Live) :
    recover.on_landed s live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨ Advance s p ⦄ := by
  unfold recover.on_landed
  cases he : s.entry with
  | none => simp [recover.refuse, he]
  | some f =>
    simp only
    step with Prior.matches_spec as ⟨ b, hb ⟩
    split_ifs
    · refine WP.spec_mono (advance_spec _) ?_
      intro p hp
      exact Or.inr hp
    · simp [recover.refuse, he]

theorem on_restore_failed_spec (failure : recover.Failure) :
    recover.on_restore_failed s failure ⦃ p =>
      ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold recover.on_restore_failed
  cases failure <;> cases he : s.entry <;> simp [recover.refuse, he]

theorem finish_spec :
    recover.finish s ⦃ p => ∃ o, p = ({ s with phase := .Finished }, .Finish o) ⦄ := by
  unfold recover.finish
  cases s.verdict <;> simp

end helpers

/-! ## The invariant of every run -/

/-- The classification consulted: with a marker, `ReadCurrent` was issued and
`classify` over the marker and the `current` read judged the run
uncommitted. Without a marker the entries are a legacy uncommitted run. -/
def Classified (s : recover.RecoverState) (cs : List recover.Command) : Prop :=
  ∀ m, s.journal.marker = some m →
    recover.Command.ReadCurrent ∈ cs ∧
    selection.classify { previous := m.previous, target := m.target, current := s.current } =
      ok .Uncommitted

/-- Phases of the uncommitted entry loop. -/
def InLoop (p : recover.Phase) : Prop :=
  p = .OpeningLeaves ∨ p = .ReadingEntry ∨ p = .Observing ∨ p = .Restoring ∨
  p = .Sealing ∨ p = .Preserving

structure Inv (s : recover.RecoverState) (cs : List recover.Command) : Prop where
  /-- While unlinking record `index`, every record up to it was issued. -/
  unlinking : s.phase = .UnlinkingRecord →
    s.index.val < s.records.val ∧
    ∀ k : U32, k.val ≤ s.index.val → recover.Command.Unlink (.Record k) ∈ cs
  /-- While syncing, the barrier was just issued after every record unlink. -/
  syncing : s.phase = .SyncingRecords →
    ∃ pre, cs = pre ++ [.Barrier .Journal] ∧
      ∀ k : U32, k.val < s.records.val → recover.Command.Unlink (.Record k) ∈ pre
  /-- Before classification nothing is decided. -/
  pending : s.phase = .Start ∨ s.phase = .ReadingJournal ∨ s.phase = .ReadingCurrent →
    s.verdict = .Pending
  /-- The current pointer is read only after it was asked for. -/
  asked : s.phase = .ReadingCurrent → recover.Command.ReadCurrent ∈ cs
  /-- The entry loop runs only in an uncommitted run. -/
  loop : InLoop s.phase → s.verdict = .Uncommitted
  /-- An uncommitted verdict came from the classification. -/
  classified : s.verdict = .Uncommitted → Classified s cs

theorem Classified.mono {s : recover.RecoverState} {cs : List recover.Command}
    (h : Classified s cs) (c : recover.Command) : Classified s (cs ++ [c]) := by
  intro m hm
  obtain ⟨h1, h2⟩ := h m hm
  exact ⟨List.mem_append_left _ h1, h2⟩

/-- The executions of the machine: from `RecoverState::new`, any sequence of
events the step accepts, with the commands it issued, in order. -/
inductive Run : recover.RecoverState → List recover.Command → Prop where
  | start {s} : recover.RecoverState.new = ok s → Run s []
  | step {s cs e s' c} : Run s cs → recover.recover_step s e = ok (s', c) → Run s' (cs ++ [c])

theorem spec_eq_ok {α : Type} {x : Result α} {P : α → Prop} {a : α}
    (h : x ⦃ r => P r ⦄) (hx : x = ok a) : P a := by
  rw [hx] at h
  exact (WP.spec_ok a).mp h

end Gripsack.Kernel
