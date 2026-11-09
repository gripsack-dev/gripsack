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

/-! ## One step keeps the invariant -/

/-- A command that is neither the marker unlink nor a mutation. -/
def Quiet (c : recover.Command) : Prop :=
  c ≠ .Unlink .Marker ∧ ∀ a b x y, c ≠ .Mutate a b x y

/-- What one step guarantees, given the invariant before it: the invariant
after it; a marker unlink only out of `SyncingRecords`; a mutation only out
of the observation of the entry under decision, restorable, with `expect`
the observed object. -/
def StepPost (s : recover.RecoverState) (e : recover.Event) (cs : List recover.Command)
    (p : recover.RecoverState × recover.Command) : Prop :=
  Inv p.1 (cs ++ [p.2]) ∧
  (p.2 = .Unlink .Marker → s.phase = .SyncingRecords) ∧
  (∀ entry dest expect effect, p.2 = .Mutate entry dest expect effect →
    s.phase = .Observing ∧ ∃ f, s.entry = some f ∧ e = .Observed dest expect ∧ f.dest = dest ∧
      entry = s.index ∧ effect = .Restore f.prior ∧ Restorable f expect)

theorem StepPost.of_quiet {s e cs p} (hi : Inv p.1 (cs ++ [p.2])) (hq : Quiet p.2) :
    StepPost s e cs p :=
  ⟨hi, fun h => absurd h hq.1, fun a b x y h => absurd h (hq.2 a b x y)⟩

/-- A step that leaves journal, current pointer and verdict alone and moves
to a phase with no bookkeeping of its own keeps the invariant. -/
theorem Inv.quiet {s : recover.RecoverState} {cs : List recover.Command} (h : Inv s cs)
    (s' : recover.RecoverState) (c : recover.Command)
    (hj : s'.journal = s.journal) (hcur : s'.current = s.current) (hv : s'.verdict = s.verdict)
    (h1 : s'.phase ≠ .UnlinkingRecord) (h2 : s'.phase ≠ .SyncingRecords)
    (h3 : s'.phase ≠ .Start) (h4 : s'.phase ≠ .ReadingJournal) (h5 : s'.phase ≠ .ReadingCurrent)
    (h6 : InLoop s'.phase → s.verdict = .Uncommitted) : Inv s' (cs ++ [c]) where
  unlinking := fun h => absurd h h1
  syncing := fun h => absurd h h2
  pending := fun h => by rcases h with h | h | h <;> contradiction
  asked := fun h => absurd h h5
  loop := fun h => hv ▸ h6 h
  classified := fun hu m hm => by
    obtain ⟨a, b⟩ := h.classified (hv ▸ hu) m (hj ▸ hm)
    exact ⟨List.mem_append_left _ a, hcur ▸ b⟩

/-- Entering cleanup at `t.index` with every earlier record already
unlinked keeps the invariant. -/
theorem Inv.of_next_unlink {t : recover.RecoverState} {cs : List recover.Command}
    {p : recover.RecoverState × recover.Command} (hn : NextUnlink t p)
    (hprev : ∀ k : U32, k.val < t.index.val → recover.Command.Unlink (.Record k) ∈ cs)
    (hcl : t.verdict = .Uncommitted → Classified t cs) : Inv p.1 (cs ++ [p.2]) := by
  rcases hn with ⟨hlt, rfl⟩ | ⟨hle, rfl⟩
  · refine ⟨fun _ => ⟨hlt, fun k hk => ?_⟩, by simp, by simp, by simp, by simp [InLoop],
      fun hu => (hcl hu).mono _⟩
    by_cases hk' : k.val < t.index.val
    · exact List.mem_append_left _ (hprev k hk')
    · have : k = t.index := UScalar.eq_of_val_eq (by simp only at hk; omega)
      subst this
      simp
  · refine ⟨by simp, fun _ => ⟨cs, rfl, fun k hk => hprev k (by simp only at hk; omega)⟩, by simp, by simp,
      by simp [InLoop], fun hu => (hcl hu).mono _⟩

theorem NextUnlink.quiet {t p} (hn : NextUnlink t p) : Quiet p.2 := by
  rcases hn with ⟨_, rfl⟩ | ⟨_, rfl⟩ <;> simp [Quiet]

/-- `proceed` from an uncommitted, classified state keeps the invariant. -/
theorem Inv.of_proceed {t : recover.RecoverState} {cs : List recover.Command}
    {p : recover.RecoverState × recover.Command} (hp : Proceed t p)
    (hv : t.verdict = .Uncommitted) (hcl : Classified t cs) :
    Inv p.1 (cs ++ [p.2]) ∧ Quiet p.2 := by
  have hq : ∀ (s' : recover.RecoverState) c, s'.journal = t.journal → s'.current = t.current →
      s'.verdict = t.verdict → s'.phase ≠ .UnlinkingRecord → s'.phase ≠ .SyncingRecords →
      s'.phase ≠ .Start → s'.phase ≠ .ReadingJournal → s'.phase ≠ .ReadingCurrent →
      Inv s' (cs ++ [c]) := by
    intro s' c hj hcur hvv h1 h2 h3 h4 h5
    refine ⟨fun h => absurd h h1, fun h => absurd h h2,
      fun h => by rcases h with h | h | h <;> contradiction, fun h => absurd h h5,
      fun _ => hvv ▸ hv, fun _ m hm => ?_⟩
    obtain ⟨a, b⟩ := hcl m (hj ▸ hm)
    exact ⟨List.mem_append_left _ a, hcur ▸ b⟩
  rcases hp with rfl | rfl | rfl | hn
  · exact ⟨hq _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp), by simp [Quiet]⟩
  · exact ⟨hq _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp), by simp [Quiet]⟩
  · exact ⟨hq _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp), by simp [Quiet]⟩
  · exact ⟨Inv.of_next_unlink hn (fun k hk => by simp at hk) (fun _ => hcl), hn.quiet⟩

theorem Inv.of_advance {t : recover.RecoverState} {cs : List recover.Command}
    {p : recover.RecoverState × recover.Command} (h : Inv t cs) (hp : Advance t p)
    (hv : t.verdict = .Uncommitted) : Inv p.1 (cs ++ [p.2]) ∧ Quiet p.2 := by
  rcases hp with rfl | ⟨i, -, hp⟩
  · exact ⟨h.quiet _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp)
      (by simp [InLoop]), by simp [Quiet]⟩
  · exact Inv.of_proceed hp hv (h.classified hv)

theorem post_abort {s : recover.RecoverState} {e cs} (h : Inv s cs) (r : recover.Refusal) :
    StepPost s e cs ({ s with phase := .Aborted }, .Abort r) :=
  StepPost.of_quiet (h.quiet _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp)
    (by simp [InLoop])) (by simp [Quiet])

theorem post_failed {s : recover.RecoverState} {e cs} (h : Inv s cs) (ev : recover.Event) :
    recover.failed_or_protocol s ev ⦃ p => StepPost s e cs p ⦄ :=
  WP.spec_mono (failed_or_protocol_spec s ev) (by rintro p ⟨r, rfl⟩; exact post_abort h r)

theorem post_refuse {s : recover.RecoverState} {e cs} (h : Inv s cs) (r : recover.Refusal) :
    recover.refuse s r ⦃ p => StepPost s e cs p ⦄ :=
  WP.spec_mono (refuse_spec s r) (by rintro p rfl; exact post_abort h r)

/-- A step into a phase with no bookkeeping that keeps journal, current and
verdict. -/
macro "quiet_post" h:ident : tactic => `(tactic|
  exact StepPost.of_quiet
    (Inv.quiet $h _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp)
      (by simp [InLoop]))
    (by simp [Quiet]))

/-- The same, into a phase of the entry loop (from one). -/
macro "loop_post" h:ident : tactic => `(tactic|
  exact StepPost.of_quiet
    (Inv.quiet $h _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp)
      (fun _ => Inv.loop $h (by simp [InLoop])))
    (by simp [Quiet]))

theorem step_spec (s : recover.RecoverState) (e : recover.Event) (cs : List recover.Command)
    (h : Inv s cs) : recover.recover_step s e ⦃ p => StepPost s e cs p ⦄ := by
  rcases s with ⟨phase, journal, records, current, verdict, index, leaves, entry⟩
  unfold recover.recover_step
  cases phase <;> cases e
  all_goals try first
    | with_reducible exact post_failed h _
    | with_reducible exact post_refuse h _
  case Start.Start =>
    simp only [WP.spec_ok]
    refine StepPost.of_quiet ⟨by simp, by simp, fun _ => h.pending (by simp), by simp,
      by simp [InLoop], fun hu => (h.classified hu).mono _⟩ (by simp [Quiet])
  case ReadingJournal.NoJournal =>
    simp only [WP.spec_ok]
    quiet_post h
  case ReadingJournal.JournalRead j =>
    simp only
    have hpend : verdict = .Pending := h.pending (by simp)
    refine WP.spec_mono (on_journal_spec _ j) ?_
    rintro p (rfl | ⟨records, hm, rfl⟩ | ⟨records, m, hm, rfl⟩ | ⟨records, hm, hp⟩)
    · exact post_abort h _
    · refine StepPost.of_quiet ⟨by simp, by simp, by simp, by simp, by simp [InLoop],
        fun hu => by simp [hpend] at hu⟩ (by simp [Quiet])
    · refine StepPost.of_quiet ⟨by simp, by simp, fun _ => by simpa using hpend, fun _ => by simp,
        by simp [InLoop], fun hu => by simp [hpend] at hu⟩ (by simp [Quiet])
    · have ⟨hi, hq⟩ := Inv.of_proceed (cs := cs) hp rfl (fun m hm' => by simp [hm] at hm')
      exact StepPost.of_quiet hi hq
  case ReadingCurrent.CurrentRead cur =>
    simp only
    have hpend : verdict = .Pending := h.pending (by simp)
    have hasked : recover.Command.ReadCurrent ∈ cs := h.asked rfl
    refine WP.spec_mono (on_current_spec _ cur) ?_
    rintro p (⟨hm, rfl⟩ | ⟨m, hm, ⟨hc, rfl⟩ | ⟨hc, hp⟩ | rfl⟩)
    · refine StepPost.of_quiet ⟨by simp, by simp, by simp, by simp, by simp [InLoop],
        fun hu => by simp [hpend] at hu⟩ (by simp [Quiet])
    · refine StepPost.of_quiet ⟨by simp, by simp, by simp, by simp, by simp [InLoop],
        fun hu => by simp at hu⟩ (by simp [Quiet])
    · have ⟨hi, hq⟩ := Inv.of_proceed (cs := cs) hp rfl (fun m' hm' => by
        simp only at hm hm'
        rw [hm] at hm'
        cases hm'
        exact ⟨hasked, hc⟩)
      exact StepPost.of_quiet hi hq
    · refine StepPost.of_quiet ⟨by simp, by simp, by simp, by simp, by simp [InLoop],
        fun hu => by simp [hpend] at hu⟩ (by simp [Quiet])
  case SealingSelection.Done =>
    simp only
    split
    · simp only [WP.spec_ok]
      quiet_post h
    · refine WP.spec_mono (begin_cleanup_spec _) ?_
      intro p hn
      exact StepPost.of_quiet
        (Inv.of_next_unlink hn (fun k hk => by simp at hk) (fun hu => h.classified hu)) hn.quiet
  case SettlingDirectories.Done =>
    refine WP.spec_mono (begin_cleanup_spec _) ?_
    intro p hn
    exact StepPost.of_quiet
      (Inv.of_next_unlink hn (fun k hk => by simp at hk) (fun hu => h.classified hu)) hn.quiet
  case ClosingLeaves.Done =>
    refine WP.spec_mono (begin_cleanup_spec _) ?_
    intro p hn
    exact StepPost.of_quiet
      (Inv.of_next_unlink hn (fun k hk => by simp at hk) (fun hu => h.classified hu)) hn.quiet
  case OpeningLeaves.Done =>
    refine WP.spec_mono (proceed_spec _) ?_
    intro p hp
    have hv : verdict = .Uncommitted := h.loop (by simp [InLoop])
    have ⟨hi, hq⟩ := Inv.of_proceed hp hv (h.classified hv)
    exact StepPost.of_quiet hi hq
  case ReadingEntry.EntryRead f =>
    have hv : verdict = .Uncommitted := h.loop (by simp [InLoop])
    refine WP.spec_mono (on_entry_spec _ f) ?_
    rintro p (⟨-, rfl⟩ | hp)
    · loop_post h
    · have ⟨hi, hq⟩ := Inv.of_advance h hp hv
      exact StepPost.of_quiet hi hq
  case Observing.Observed d live =>
    refine WP.spec_mono (on_observed_spec _ d live) ?_
    rintro p (⟨r, rfl, -⟩ | ⟨f, hf, hfd, -, ⟨hr, rfl⟩ | rfl | rfl⟩)
    · exact post_abort h r
    · refine ⟨h.quiet _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp)
        (fun _ => h.loop (by simp [InLoop])), by simp, ?_⟩
      intro a b x y hm
      simp only [recover.Command.Mutate.injEq] at hm
      obtain ⟨rfl, rfl, rfl, rfl⟩ := hm
      exact ⟨rfl, f, hf, rfl, hfd, rfl, rfl, hr⟩
    · loop_post h
    · loop_post h
  case Restoring.Landed live =>
    have hv : verdict = .Uncommitted := h.loop (by simp [InLoop])
    refine WP.spec_mono (on_landed_spec _ live) ?_
    rintro p (⟨r, rfl⟩ | hp)
    · exact post_abort h r
    · have ⟨hi, hq⟩ := Inv.of_advance h hp hv
      exact StepPost.of_quiet hi hq
  case Restoring.Failed failure =>
    exact WP.spec_mono (on_restore_failed_spec _ failure) (by rintro p ⟨r, rfl⟩; exact post_abort h r)
  case Sealing.Failed failure =>
    exact WP.spec_mono (on_restore_failed_spec _ failure) (by rintro p ⟨r, rfl⟩; exact post_abort h r)
  case Sealing.Done =>
    have hv : verdict = .Uncommitted := h.loop (by simp [InLoop])
    refine WP.spec_mono (advance_spec' _) ?_
    intro p hp
    have ⟨hi, hq⟩ := Inv.of_advance h hp hv
    exact StepPost.of_quiet hi hq
  case Preserving.Done =>
    have hv : verdict = .Uncommitted := h.loop (by simp [InLoop])
    refine WP.spec_mono (advance_spec' _) ?_
    intro p hp
    have ⟨hi, hq⟩ := Inv.of_advance h hp hv
    exact StepPost.of_quiet hi hq
  case UnlinkingRecord.Done =>
    have hun := h.unlinking rfl
    have hspec := U32.checked_add_bv_spec index 1#u32
    cases ho : U32.checked_add index 1#u32 with
    | none =>
      simp only [lift, Std.bind_ok]
      exact post_refuse h _
    | some i =>
      simp only [ho] at hspec
      simp only [lift, Std.bind_ok]
      refine WP.spec_mono (next_unlink_spec _) ?_
      intro p hn
      refine StepPost.of_quiet (Inv.of_next_unlink hn (fun k hk => ?_) (fun hu => h.classified hu))
        hn.quiet
      simp only at hk hun
      exact hun.2 k (by simp at hspec; omega)
  case SyncingRecords.Done =>
    simp only [WP.spec_ok]
    refine ⟨h.quiet _ _ rfl rfl rfl (by simp) (by simp) (by simp) (by simp) (by simp)
      (by simp [InLoop]), fun _ => rfl, by simp⟩
  case UnlinkingMarker.Done =>
    simp only [WP.spec_ok]
    quiet_post h
  case SyncingMarker.Done =>
    refine WP.spec_mono (finish_spec _) ?_
    rintro p ⟨o, rfl⟩
    quiet_post h

/-! ## The theorems -/

/-- Every execution keeps the invariant. -/
theorem run_inv {s : recover.RecoverState} {cs : List recover.Command} (h : Run s cs) :
    Inv s cs := by
  induction h with
  | start hs =>
    simp only [recover.RecoverState.new, ok.injEq] at hs
    subst hs
    exact ⟨by simp, by simp, fun _ => rfl, by simp, by simp [InLoop], by simp⟩
  | step _ hstep ih => exact (spec_eq_ok (step_spec _ _ _ ih) hstep).1

/-- (a) The K1-style guard. In every execution, a restore is issued only in
answer to the observation of the entry under decision, names that entry and
its destination, carries the observed live object as `expect` (which the
driver re-checks before acting), restores that entry's prior, and only when
the live object is the run's own write (intended or immediate pre-state) or
an absence where a prior existed. A foreign object is never mutated. -/
theorem mutate_guard {s s' : recover.RecoverState} {cs : List recover.Command}
    {e : recover.Event} {entry : U32} {dest : ids.DestId} {expect : recover.Live}
    {effect : recover.Effect} (hrun : Run s cs)
    (h : recover.recover_step s e = ok (s', .Mutate entry dest expect effect)) :
    s.phase = .Observing ∧ ∃ f, s.entry = some f ∧ e = .Observed dest expect ∧
      f.dest = dest ∧ entry = s.index ∧ effect = .Restore f.prior ∧ Restorable f expect :=
  (spec_eq_ok (step_spec _ _ _ (run_inv hrun)) h).2.2 _ _ _ _ rfl

/-- (c) An entry whose live object has no journal identity (a directory, a
special file, a non-UTF-8 link) is refused by name: the observation answers
with `Abort (Unrestorable entry dest)`, whatever state the run is in. -/
theorem unrestorable_refused (s : recover.RecoverState) (f : recover.EntryFacts)
    (d : ids.DestId) (hphase : s.phase = .Observing) (hentry : s.entry = some f)
    (hdest : f.dest = d) :
    recover.recover_step s (.Observed d .Unsupported) ⦃ p =>
      p.2 = .Abort (.Unrestorable s.index d) ∧ p.1.phase = .Aborted ⦄ := by
  rcases s with ⟨phase, journal, records, current, verdict, index, leaves, entry⟩
  simp only at hphase hentry
  subst hphase
  unfold recover.recover_step
  refine WP.spec_mono (on_observed_spec _ d .Unsupported) ?_
  rintro p (⟨r, rfl, hr⟩ | ⟨f', -, -, hu, -⟩)
  · exact ⟨by simp [hr f hentry hdest rfl], rfl⟩
  · exact absurd rfl hu

/-- (c) An object without a journal identity is never the target of a
mutation, in any execution. -/
theorem mutate_never_unsupported {s s' : recover.RecoverState} {cs : List recover.Command}
    {e : recover.Event} {entry : U32} {dest : ids.DestId} {expect : recover.Live}
    {effect : recover.Effect} (hrun : Run s cs)
    (h : recover.recover_step s e = ok (s', .Mutate entry dest expect effect)) :
    expect ≠ .Unsupported := by
  obtain ⟨-, f, -, -, -, -, -, hr⟩ := mutate_guard hrun h
  rintro rfl
  rcases hr with ⟨i, hi, -⟩ | ⟨hi, -⟩ <;> cases hi

/-- (b) Cleanup order. In every execution, the run-marker unlink is issued
only immediately after a journal barrier, and every journal record (entries
and directory records) was unlinked before that barrier. So a durable marker
removal implies durable record removals (0026 §5; the 0.19.x resurrections
are the violations). -/
theorem marker_unlink_after_record_barrier {s s' : recover.RecoverState}
    {cs : List recover.Command} {e : recover.Event} (hrun : Run s cs)
    (h : recover.recover_step s e = ok (s', .Unlink .Marker)) :
    ∃ pre, cs = pre ++ [.Barrier .Journal] ∧
      ∀ k : U32, k.val < s.records.val → recover.Command.Unlink (.Record k) ∈ pre := by
  have hi := run_inv hrun
  exact hi.syncing ((spec_eq_ok (step_spec _ _ _ hi) h).2.1 rfl)

/-- (d) Classification before restore. In every execution, a restore is
issued only in a run whose verdict is uncommitted, and that verdict came
from the commit classification: with a run marker, `ReadCurrent` was issued
earlier and `classify` (K3, CLASSIFY-001) over the marker and the `current`
read judged the run uncommitted. A committed or ambiguous run never
restores. -/
theorem mutate_after_classification {s s' : recover.RecoverState} {cs : List recover.Command}
    {e : recover.Event} {entry : U32} {dest : ids.DestId} {expect : recover.Live}
    {effect : recover.Effect} (hrun : Run s cs)
    (h : recover.recover_step s e = ok (s', .Mutate entry dest expect effect)) :
    s.verdict = .Uncommitted ∧
    ∀ m, s.journal.marker = some m →
      recover.Command.ReadCurrent ∈ cs ∧
      selection.classify { previous := m.previous, target := m.target, current := s.current } =
        ok .Uncommitted := by
  have hi := run_inv hrun
  have hv := hi.loop (by rw [(mutate_guard hrun h).1]; simp [InLoop])
  exact ⟨hv, hi.classified hv⟩

end Gripsack.Kernel
