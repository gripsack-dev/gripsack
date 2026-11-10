import Extracted.GripsackKernel
import Kernel.Recover
import Kernel.Rollback

/-!
# The journal run and the leaf effect protocol (plan/0056 F2 step 3)

Theorems over the Aeneas translation of `gripsack_kernel::journal_run`
(`Extracted/GripsackKernel.lean`). An execution of the machine is its log:
every step's event paired with the command it issued, in order (`Run`).

* P1 `mutate_after_durable_intent`: in every execution, a mutation is issued
  only in answer to `Done`, after a log that starts with the run marker's
  complete publication (stage, sync the file, publish the name, sync the
  journal directory, each acknowledged) and ends with exactly this unit's
  leaf protocol: the unit's facts answered by `Observe`, the observation —
  equal to the plan's `expect` — answered by `CapturePrior` with that
  `expect`, then the four publication steps of its entry, each answered
  `Done`. The mutation carries the observed object as `expect` (which the
  driver re-checks before acting) and the unit's intended object.
* P3 `record_unlink_needs_no_evidence`: in every execution, a journal record
  is unlinked only after the selection flip was acknowledged durable, or in
  a run that never published an entry.
* P3 `marker_unlink_after_record_barrier`: in every execution, the marker
  unlink is issued only immediately after a journal barrier, and every
  record was unlinked before that barrier (0026 §5).
* `unit_accepted_only_landed`: the machine accepts a unit (ready again) only
  when the landed object is the unit's intended one.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.JournalRun

abbrev State := journal_run.RunState
abbrev Cmd := journal_run.Command
abbrev Ev := journal_run.Event
/-- An execution: each step's event with the command it issued. -/
abbrev Log := List (Ev × Cmd)

/-- The steps of one record's publication, up to and including `a`: the
first issued in answer to `first`, every later one in answer to `Done`. -/
def pubSteps (r : journal_run.Record) (first : Ev) : journal.PublicationAction → Log
  | .Stage => [(first, .Publish r .Stage)]
  | .SyncFile => [(first, .Publish r .Stage), (.Done, .Publish r .SyncFile)]
  | .PublishName => [(first, .Publish r .Stage), (.Done, .Publish r .SyncFile),
      (.Done, .Publish r .PublishName)]
  | .SyncNamespace => [(first, .Publish r .Stage), (.Done, .Publish r .SyncFile),
      (.Done, .Publish r .PublishName), (.Done, .Publish r .SyncNamespace)]
  | .Complete => []
  | .Refused => []

/-- A publication effect (not completion or refusal). -/
def Publishing (a : journal.PublicationAction) : Prop :=
  a = .Stage ∨ a = .SyncFile ∨ a = .PublishName ∨ a = .SyncNamespace

/-- The run's log starts with the marker's complete publication, then
`Ready` in answer to its namespace barrier. -/
def Marked (log : Log) : Prop :=
  ∃ rest, log = pubSteps .Marker .Start .SyncNamespace ++ [(.Done, .Ready)] ++ rest

/-- One unit's leaf protocol up to its entry's publication step `a`. -/
def leafLog (f : journal_run.UnitFacts) (u : U32) (x : recover.Live)
    (a : journal.PublicationAction) : Log :=
  [(.Unit f, .Observe u f.dest), (.Observed f.dest x, .CapturePrior u f.dest x)] ++
    pubSteps .Entry .Done a

/-- No entry publication was ever issued. -/
def NoEntry (log : Log) : Prop := ∀ e a, (e, journal_run.Command.Publish .Entry a) ∉ log

/-- The flip was issued, and acknowledged by the step that settled the
committed run. -/
def Flipped (log : Log) : Prop :=
  ∃ pre rest, log = pre ++ [(.Commit, .Flip), (.Done, .Settle .Committed)] ++ rest

def Issued (c : Cmd) (log : Log) : Prop := ∃ e, (e, c) ∈ log

/-- Phases after the marker's publication. -/
def Past (p : journal_run.Phase) : Prop :=
  p = .Ready ∨ p = .Observing ∨ p = .Capturing ∨ p = .PublishingEntry ∨ p = .Mutating ∨
  p = .Flipping ∨ p = .Settling ∨ p = .UnlinkingRecord ∨ p = .SyncingRecords ∨
  p = .UnlinkingMarker ∨ p = .SyncingMarker ∨ p = .Finished

/-- Phases of the completion. -/
def InCleanup (p : journal_run.Phase) : Prop :=
  p = .Settling ∨ p = .UnlinkingRecord ∨ p = .SyncingRecords ∨ p = .UnlinkingMarker ∨
  p = .SyncingMarker

structure Inv (s : State) (log : Log) : Prop where
  start : s.phase = .Start → log = []
  marker : s.phase = .PublishingMarker → Publishing s.step ∧ log = pubSteps .Marker .Start s.step
  marked : Past s.phase → Marked log
  observing : s.phase = .Observing →
    ∃ pre f, s.unit = some f ∧ log = pre ++ [(.Unit f, .Observe s.units f.dest)]
  capturing : s.phase = .Capturing →
    ∃ pre f, s.unit = some f ∧ f.expect = s.observed ∧
      log = pre ++ [(.Unit f, .Observe s.units f.dest),
        (.Observed f.dest s.observed, .CapturePrior s.units f.dest s.observed)]
  entry : s.phase = .PublishingEntry → Publishing s.step ∧
    ∃ pre f, s.unit = some f ∧ f.expect = s.observed ∧
      log = pre ++ leafLog f s.units s.observed s.step
  flipping : s.phase = .Flipping → ∃ pre, log = pre ++ [(.Commit, .Flip)]
  committed : s.committed = true → Flipped log
  quiet : s.phase = .Ready → s.units.val = 0 → NoEntry log
  cleanup : InCleanup s.phase → s.committed = true ∨ (s.units.val = 0 ∧ NoEntry log)
  unlinking : s.phase = .UnlinkingRecord →
    s.index.val < s.records.val ∧ ∀ k : U32, k.val ≤ s.index.val → Issued (.Unlink (.Record k)) log
  syncing : s.phase = .SyncingRecords →
    ∃ pre e, log = pre ++ [(e, .Barrier)] ∧
      ∀ k : U32, k.val < s.records.val → Issued (.Unlink (.Record k)) pre

/-- The executions of the machine: from `RunState::new`, any sequence of
events the step accepts, logged with the commands it issued. -/
inductive Run : State → Log → Prop where
  | start {s} : journal_run.RunState.new = ok s → Run s []
  | step {s log e s' c} : Run s log → journal_run.run_step s e = ok (s', c) →
      Run s' (log ++ [(e, c)])

/-! ## Monotonicity of the log predicates -/

theorem Marked.mono {log : Log} (h : Marked log) (x : Ev × Cmd) : Marked (log ++ [x]) := by
  obtain ⟨rest, rfl⟩ := h
  exact ⟨rest ++ [x], by simp⟩

theorem Flipped.mono {log : Log} (h : Flipped log) (x : Ev × Cmd) : Flipped (log ++ [x]) := by
  obtain ⟨pre, rest, rfl⟩ := h
  exact ⟨pre, rest ++ [x], by simp⟩

theorem NoEntry.mono {log : Log} (h : NoEntry log) (x : Ev × Cmd)
    (hx : ∀ e a, x ≠ (e, journal_run.Command.Publish .Entry a)) : NoEntry (log ++ [x]) := by
  intro e a hm
  rcases List.mem_append.mp hm with hm | hm
  · exact h e a hm
  · simp only [List.mem_singleton] at hm
    exact hx e a hm.symm

theorem Issued.mono {c : Cmd} {log : Log} (h : Issued c log) (x : Ev × Cmd) :
    Issued c (log ++ [x]) := by
  obtain ⟨e, he⟩ := h
  exact ⟨e, List.mem_append_left _ he⟩

/-! ## What the step helpers return -/

section helpers
variable (s : State)

theorem refuse_spec (r : journal_run.Refusal) :
    journal_run.refuse s r ⦃ p => p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  simp [journal_run.refuse]

theorem failed_or_protocol_spec (e : Ev) :
    journal_run.failed_or_protocol s e ⦃ p =>
      ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold journal_run.failed_or_protocol
  cases e <;> simp [journal_run.refuse]

theorem follows_spec (prev : Option recover.Intended) (l : recover.Live) :
    journal_run.follows prev l ⦃ _ => True ⦄ := by
  unfold journal_run.follows
  cases prev with
  | none => simp
  | some i => exact WP.spec_mono (Rollback.lands_spec i l) (fun _ _ => trivial)

theorem on_observed_spec (d : ids.DestId) (live : recover.Live) :
    journal_run.on_observed s d live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f, s.unit = some f ∧ f.dest = d ∧ f.expect = live ∧
        p = ({ s with phase := .Capturing, observed := live }, .CapturePrior s.units d live) ⦄ := by
  unfold journal_run.on_observed
  cases hu : s.unit with
  | none => simp [journal_run.refuse, hu]
  | some f =>
    simp only
    step with DestId.same_spec as ⟨ b, hb ⟩
    split_ifs with hd
    · have hfd : f.dest = d := hb.mp hd
      cases live with
      | Unsupported => simp [journal_run.refuse, hu]
      | Absent =>
        simp only
        step with Live.same_spec as ⟨ b1, hb1 ⟩
        split_ifs with h1
        · step with follows_spec as ⟨ b2 ⟩
          split_ifs
          · simp only [WP.spec_ok]
            exact Or.inr ⟨f, rfl, hfd, (hb1.mp h1).symm, by simp⟩
          · simp [journal_run.refuse, hu]
        · simp [journal_run.refuse, hu]
      | Obj o =>
        simp only
        step with Live.same_spec as ⟨ b1, hb1 ⟩
        split_ifs with h1
        · step with follows_spec as ⟨ b2 ⟩
          split_ifs
          · simp only [WP.spec_ok]
            exact Or.inr ⟨f, rfl, hfd, (hb1.mp h1).symm, by simp⟩
          · simp [journal_run.refuse, hu]
        · simp [journal_run.refuse, hu]
    · simp [journal_run.refuse, hu]

theorem on_act_failed_spec (failure : recover.Failure) :
    journal_run.on_act_failed s failure ⦃ p =>
      ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold journal_run.on_act_failed
  cases failure <;> cases hu : s.unit <;> simp [journal_run.refuse, hu]

theorem on_landed_spec (live : recover.Live) :
    journal_run.on_landed s live ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ f u, s.unit = some f ∧ Rollback.Lands f.intended live ∧ u.val = s.units.val + 1 ∧
        p = ({ s with phase := .Ready, units := u, unit := none }, .Ready) ⦄ := by
  unfold journal_run.on_landed
  cases hu : s.unit with
  | none => simp [journal_run.refuse, hu]
  | some f =>
    simp only
    step with Rollback.lands_spec as ⟨ b, hb ⟩
    split_ifs with hl
    · have h := U32.checked_add_bv_spec s.units 1#u32
      cases ho : U32.checked_add s.units 1#u32 with
      | none => simp [lift, journal_run.refuse, hu]
      | some u =>
        simp only [ho] at h
        simp only [lift, Std.bind_ok, WP.spec_ok]
        exact Or.inr ⟨f, u, rfl, hb.mp hl, by simpa using h.2.1, rfl⟩
    · simp [journal_run.refuse, hu]

theorem on_end_spec :
    journal_run.on_end s ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      (s.units.val = 0 ∧ p = ({ s with phase := .Settling }, .Settle .Ended)) ⦄ := by
  unfold journal_run.on_end
  split_ifs with h
  · simp [journal_run.refuse]
  · simp only [bne_iff_ne, ne_eq, not_not] at h
    simp [h]

/-- How cleanup continues from `s`: the next record unlink, or the journal
barrier once every record was issued. -/
def NextUnlink (p : State × Cmd) : Prop :=
  (s.index.val < s.records.val ∧
    p = ({ s with phase := .UnlinkingRecord }, .Unlink (.Record s.index))) ∨
  (s.records.val ≤ s.index.val ∧ p = ({ s with phase := .SyncingRecords }, .Barrier))

theorem next_unlink_spec : journal_run.next_unlink s ⦃ p => NextUnlink s p ⦄ := by
  unfold journal_run.next_unlink NextUnlink
  split_ifs with h
  · simp only [WP.spec_ok]; left; exact ⟨by scalar_tac, by simp⟩
  · simp only [WP.spec_ok]; right; exact ⟨by scalar_tac, by simp⟩

theorem finish_spec :
    journal_run.finish s ⦃ p => ∃ o, p = ({ s with phase := .Finished }, .Finish o) ⦄ := by
  unfold journal_run.finish
  split_ifs <;> simp

end helpers

/-! ## One step keeps the invariant -/

/-- What one step guarantees, given the invariant before it. -/
structure StepPost (s : State) (e : Ev) (log : Log) (p : State × Cmd) : Prop where
  inv : Inv p.1 (log ++ [(e, p.2)])
  mutate : ∀ u d x i, p.2 = .Mutate u d x i →
    s.phase = .PublishingEntry ∧ s.step = .SyncNamespace ∧ e = .Done ∧
    ∃ f, s.unit = some f ∧ u = s.units ∧ d = f.dest ∧ x = s.observed ∧ i = f.intended
  record : ∀ k, p.2 = .Unlink (.Record k) → s.phase = .Settling ∨ s.phase = .UnlinkingRecord
  marker : p.2 = .Unlink .Marker → s.phase = .SyncingRecords
  ready : p.2 = .Ready → s.phase = .Mutating →
    ∃ l f, e = .Landed l ∧ s.unit = some f ∧ Rollback.Lands f.intended l

/-- An abort keeps the invariant: the aborted phase has no bookkeeping. -/
theorem post_abort {s : State} {e : Ev} {log : Log} (h : Inv s log) (r : journal_run.Refusal) :
    StepPost s e log ({ s with phase := .Aborted }, .Abort r) where
  inv := {
    start := by simp
    marker := by simp
    marked := by simp [Past]
    observing := by simp
    capturing := by simp
    entry := by simp
    flipping := by simp
    committed := fun hc => (h.committed hc).mono _
    quiet := by simp
    cleanup := by simp [InCleanup]
    unlinking := by simp
    syncing := by simp }
  mutate := by simp
  record := by simp
  marker := by simp
  ready := by simp

theorem post_failed {s : State} {e : Ev} {log : Log} (h : Inv s log) (ev : Ev) :
    journal_run.failed_or_protocol s ev ⦃ p => StepPost s e log p ⦄ :=
  WP.spec_mono (failed_or_protocol_spec s ev) (by rintro p ⟨r, rfl⟩; exact post_abort h r)

theorem post_refuse {s : State} {e : Ev} {log : Log} (h : Inv s log) (r : journal_run.Refusal) :
    journal_run.refuse s r ⦃ p => StepPost s e log p ⦄ :=
  WP.spec_mono (refuse_spec s r) (by rintro p rfl; exact post_abort h r)

/-- Entering or continuing the cleanup with every earlier record issued. -/
theorem post_next_unlink {s t : State} {e : Ev} {log : Log} {p : State × Cmd}
    (hn : NextUnlink t p) (hfrom : s.phase = .Settling ∨ s.phase = .UnlinkingRecord)
    (hprev : ∀ k : U32, k.val < t.index.val → Issued (.Unlink (.Record k)) log)
    (hcom : t.committed = true → Flipped log) (hmk : Marked log)
    (hcl : t.committed = true ∨ (t.units.val = 0 ∧ NoEntry log)) :
    StepPost s e log p := by
  have hcl' : ∀ x : Ev × Cmd, (∀ e a, x ≠ (e, journal_run.Command.Publish .Entry a)) →
      t.committed = true ∨ (t.units.val = 0 ∧ NoEntry (log ++ [x])) := by
    intro x hx
    rcases hcl with hc | ⟨hu, hne⟩
    · exact Or.inl hc
    · exact Or.inr ⟨hu, hne.mono x hx⟩
  rcases hn with ⟨hlt, rfl⟩ | ⟨hle, rfl⟩
  · refine ⟨{
      start := by simp
      marker := by simp
      marked := fun _ => hmk.mono _
      observing := by simp
      capturing := by simp
      entry := by simp
      flipping := by simp
      committed := fun hc => (hcom hc).mono _
      quiet := by simp
      cleanup := fun _ => hcl' _ (by simp)
      unlinking := fun _ => ⟨hlt, fun k hk => ?_⟩
      syncing := by simp }, by simp, fun _ _ => hfrom, by simp, by simp⟩
    by_cases hk' : k.val < t.index.val
    · exact (hprev k hk').mono _
    · have : k = t.index := UScalar.eq_of_val_eq (by simp only at hk; omega)
      subst this
      exact ⟨e, by simp⟩
  · refine ⟨{
      start := by simp
      marker := by simp
      marked := fun _ => hmk.mono _
      observing := by simp
      capturing := by simp
      entry := by simp
      flipping := by simp
      committed := fun hc => (hcom hc).mono _
      quiet := by simp
      cleanup := fun _ => hcl' _ (by simp)
      unlinking := by simp
      syncing := fun _ => ⟨log, e, rfl, fun k hk => hprev k (by simp only at hk; omega)⟩ },
      by simp, by simp, by simp, by simp⟩

/-- Discharge the clauses a concrete step makes vacuous: a phase or command
the clause does not speak about. -/
macro "vac" : tactic => `(tactic| first
  | (intro h; simp [Past, InCleanup] at h; done)
  | (intro _ h; simp [Past, InCleanup] at h; done)
  | (intro _ _ h; simp at h; done)
  | (intro _ _ _ _ h; simp at h; done))

/-- Open a `StepPost` for a concrete step: every clause as a goal, the
vacuous ones discharged. -/
macro "post" : tactic => `(tactic| (
  refine ⟨⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩, ?_, ?_, ?_, ?_⟩
  all_goals try vac))

theorem step_spec (s : State) (e : Ev) (log : Log) (h : Inv s log) :
    journal_run.run_step s e ⦃ p => StepPost s e log p ⦄ := by
  rcases s with ⟨phase, step, committed, units, unit, observed, records, index⟩
  unfold journal_run.run_step
  cases phase <;> cases e
  all_goals try first
    | with_reducible exact post_failed h _
    | with_reducible exact post_refuse h _
  case Start.Start =>
    have hl := h.start rfl
    subst hl
    simp only [WP.spec_ok]
    post
    all_goals first
      | exact fun hc => (h.committed hc).mono _
      | exact fun _ => ⟨Or.inl rfl, rfl⟩
  case PublishingMarker.Done =>
    obtain ⟨hp, hl⟩ := h.marker rfl
    subst hl
    unfold journal_run.on_marker_step
    rcases hp with rfl | rfl | rfl | rfl <;>
      simp only [journal.PublicationAction.successor, Std.bind_ok, WP.spec_ok] <;> post
    all_goals first
      | exact fun hc => (h.committed hc).mono _
      | exact fun _ => ⟨by simp [Publishing], by simp [pubSteps]⟩
      | exact fun _ => ⟨[], by simp [pubSteps]⟩
      | exact fun _ _ => by simp [NoEntry, pubSteps]
  case Ready.Unit f =>
    have hm := h.marked (by simp [Past])
    simp only [journal_run.on_unit, WP.spec_ok]
    post
    all_goals first
      | exact fun _ => hm.mono _
      | exact fun hc => (h.committed hc).mono _
      | exact fun _ => ⟨log, f, rfl, rfl⟩
  case Ready.Commit =>
    have hm := h.marked (by simp [Past])
    simp only [WP.spec_ok]
    post
    all_goals first
      | exact fun _ => hm.mono _
      | exact fun hc => (h.committed hc).mono _
      | exact fun _ => ⟨log, rfl⟩
  case Ready.End =>
    have hm := h.marked (by simp [Past])
    refine WP.spec_mono (on_end_spec _) ?_
    rintro p (⟨r, rfl⟩ | ⟨hu, rfl⟩)
    · exact post_abort h r
    · post
      all_goals first
        | exact fun _ => hm.mono _
        | exact fun hc => (h.committed hc).mono _
        | exact fun _ => Or.inr ⟨hu, (h.quiet rfl hu).mono _ (by simp)⟩
  case Observing.Observed d l =>
    have hm := h.marked (by simp [Past])
    obtain ⟨pre, f', hu', hl⟩ := h.observing rfl
    refine WP.spec_mono (on_observed_spec _ d l) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, hu, hd, hx, rfl⟩)
    · exact post_abort h r
    · simp only at hu hu'
      rw [hu'] at hu
      cases hu
      subst hd hl
      post
      all_goals first
        | exact fun _ => hm.mono _
        | exact fun hc => (h.committed hc).mono _
        | exact fun _ => ⟨pre, f', by simpa using hu', hx, by simp⟩
  case Capturing.Done =>
    have hm := h.marked (by simp [Past])
    obtain ⟨pre, f, hu, hx, hl⟩ := h.capturing rfl
    subst hl
    simp only [journal_run.on_captured, WP.spec_ok]
    post
    all_goals first
      | exact fun _ => hm.mono _
      | exact fun hc => (h.committed hc).mono _
      | exact fun _ => ⟨Or.inl rfl, pre, f, by simpa using hu, by simpa using hx,
          by simp [leafLog, pubSteps]⟩
  case Capturing.Failed failure =>
    exact WP.spec_mono (on_act_failed_spec _ failure) (by rintro p ⟨r, rfl⟩; exact post_abort h r)
  case PublishingEntry.Done =>
    have hm := h.marked (by simp [Past])
    obtain ⟨hp, pre, f, hu, hx, hl⟩ := h.entry rfl
    subst hl
    simp only at hu
    unfold journal_run.on_entry_step
    simp only [hu]
    rcases hp with rfl | rfl | rfl | rfl <;>
      simp only [journal.PublicationAction.successor, Std.bind_ok, WP.spec_ok] <;> post
    all_goals first
      | exact fun _ => hm.mono _
      | exact fun hc => (h.committed hc).mono _
      | exact fun _ => ⟨by simp [Publishing], pre, f, rfl, hx, by simp [leafLog, pubSteps]⟩
      | (intro u d x i hc
         simp only [journal_run.Command.Mutate.injEq] at hc
         obtain ⟨rfl, rfl, rfl, rfl⟩ := hc
         exact ⟨rfl, rfl, rfl, f, rfl, rfl, rfl, rfl, rfl⟩)
  case Mutating.Landed l =>
    have hm := h.marked (by simp [Past])
    refine WP.spec_mono (on_landed_spec _ l) ?_
    rintro p (⟨r, rfl⟩ | ⟨f, u, hu, hl, hv, rfl⟩)
    · exact post_abort h r
    · post
      all_goals first
        | exact fun _ => hm.mono _
        | exact fun hc => (h.committed hc).mono _
        | exact fun _ h0 => by simp only at h0; omega
        | exact fun _ _ => ⟨l, f, rfl, by simpa using hu, hl⟩
  case Mutating.Failed failure =>
    exact WP.spec_mono (on_act_failed_spec _ failure) (by rintro p ⟨r, rfl⟩; exact post_abort h r)
  case Flipping.Done =>
    have hm := h.marked (by simp [Past])
    obtain ⟨pre, hl⟩ := h.flipping rfl
    subst hl
    simp only [WP.spec_ok]
    post
    all_goals first
      | exact fun _ => hm.mono _
      | exact fun _ => ⟨pre, [], by simp⟩
      | exact fun _ => Or.inl rfl
  case Settling.Settled r =>
    refine WP.spec_mono (next_unlink_spec _) ?_
    intro p hn
    exact post_next_unlink hn (Or.inl rfl) (fun k hk => by simp at hk) h.committed
      (h.marked (by simp [Past])) (h.cleanup (by simp [InCleanup]))
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
      refine post_next_unlink hn (Or.inr rfl) (fun k hk => ?_) h.committed
        (h.marked (by simp [Past])) (h.cleanup (by simp [InCleanup]))
      simp only at hk hun
      exact hun.2 k (by simp at hspec; omega)
  case SyncingRecords.Done =>
    have hm := h.marked (by simp [Past])
    have hcl : ∀ x : Ev × Cmd, (∀ e a, x ≠ (e, journal_run.Command.Publish .Entry a)) →
        committed = true ∨ (units.val = 0 ∧ NoEntry (log ++ [x])) := by
      intro x hx
      rcases h.cleanup (by simp [InCleanup]) with hc | ⟨hu, hn⟩
      · exact Or.inl hc
      · exact Or.inr ⟨hu, hn.mono x hx⟩
    simp only [WP.spec_ok]
    post
    all_goals first
      | exact fun _ => hm.mono _
      | exact fun hc => (h.committed hc).mono _
      | exact fun _ => hcl _ (by simp)
      | exact fun _ => rfl
  case UnlinkingMarker.Done =>
    have hm := h.marked (by simp [Past])
    have hcl : ∀ x : Ev × Cmd, (∀ e a, x ≠ (e, journal_run.Command.Publish .Entry a)) →
        committed = true ∨ (units.val = 0 ∧ NoEntry (log ++ [x])) := by
      intro x hx
      rcases h.cleanup (by simp [InCleanup]) with hc | ⟨hu, hn⟩
      · exact Or.inl hc
      · exact Or.inr ⟨hu, hn.mono x hx⟩
    simp only [WP.spec_ok]
    post
    all_goals first
      | exact fun _ => hm.mono _
      | exact fun hc => (h.committed hc).mono _
      | exact fun _ => hcl _ (by simp)
  case SyncingMarker.Done =>
    have hm := h.marked (by simp [Past])
    refine WP.spec_mono (finish_spec _) ?_
    rintro p ⟨o, rfl⟩
    post
    all_goals first
      | exact fun _ => hm.mono _
      | exact fun hc => (h.committed hc).mono _

/-! ## The theorems -/

/-- Every execution keeps the invariant. -/
theorem run_inv {s : State} {log : Log} (h : Run s log) : Inv s log := by
  induction h with
  | start hs =>
    simp only [journal_run.RunState.new, ok.injEq] at hs
    subst hs
    exact ⟨fun _ => rfl, by simp, by simp [Past], by simp, by simp, by simp, by simp, by simp,
      by simp, by simp [InCleanup], by simp, by simp⟩
  | step _ hstep ih => exact (spec_eq_ok (step_spec _ _ _ ih) hstep).inv

theorem run_post {s s' : State} {log : Log} {e : Ev} {c : Cmd} (hrun : Run s log)
    (h : journal_run.run_step s e = ok (s', c)) : StepPost s e log (s', c) :=
  spec_eq_ok (step_spec _ _ _ (run_inv hrun)) h

/-- P1: mutation never precedes durable intent and prior evidence. In every
execution, a mutation is issued only in answer to `Done`, after a log that
starts with the run marker's complete publication (each of its four steps
acknowledged before the next, the last one the journal directory's barrier)
and ends with exactly this unit's leaf protocol: the unit answered by
`Observe` of its destination; the observation answered by `CapturePrior`
with that observed object, which is the plan's `expect`; then stage, sync,
publish and sync of its entry, each answered `Done`. The mutation names the
unit and its destination, carries the observed object as `expect` (the
driver re-checks it before acting) and installs the unit's intended object. -/
theorem mutate_after_durable_intent {s s' : State} {log : Log} {e : Ev} {u : U32}
    {d : ids.DestId} {x : recover.Live} {i : recover.Intended} (hrun : Run s log)
    (h : journal_run.run_step s e = ok (s', .Mutate u d x i)) :
    e = .Done ∧ Marked log ∧
    ∃ pre f, log = pre ++ leafLog f u x .SyncNamespace ∧
      f.dest = d ∧ f.expect = x ∧ f.intended = i := by
  have hi := run_inv hrun
  obtain ⟨hph, hst, he, f, hu, rfl, rfl, rfl, rfl⟩ := (run_post hrun h).mutate _ _ _ _ rfl
  obtain ⟨-, pre, f', hu', hx, hl⟩ := hi.entry hph
  rw [hu] at hu'
  cases hu'
  rw [hst] at hl
  exact ⟨he, hi.marked (by simp [Past, hph]), pre, f, hl, rfl, hx, rfl⟩

/-- P3 (cleanup keeps the evidence a later crash needs): in every execution,
a journal record is unlinked only after the selection flip was issued and
acknowledged (`Done`, answered by settling the committed run) — so recovery
after any later crash classifies the run committed and needs no entry — or in
a run that never published an entry. -/
theorem record_unlink_needs_no_evidence {s s' : State} {log : Log} {e : Ev} {k : U32}
    (hrun : Run s log) (h : journal_run.run_step s e = ok (s', .Unlink (.Record k))) :
    Flipped log ∨ NoEntry log := by
  have hi := run_inv hrun
  have hph := (run_post hrun h).record k rfl
  have hcl := hi.cleanup (by rcases hph with hp | hp <;> simp [InCleanup, hp])
  rcases hcl with hc | ⟨-, hn⟩
  · exact Or.inl (hi.committed hc)
  · exact Or.inr hn

/-- P3 (cleanup order): in every execution, the run-marker unlink is issued
only immediately after a journal barrier, and every journal record was
unlinked before that barrier. So a durable marker removal implies durable
record removals (0026 §5; the 0.19.x resurrections are the violations). -/
theorem marker_unlink_after_record_barrier {s s' : State} {log : Log} {e : Ev}
    (hrun : Run s log) (h : journal_run.run_step s e = ok (s', .Unlink .Marker)) :
    ∃ pre e', log = pre ++ [(e', .Barrier)] ∧
      ∀ k : U32, k.val < s.records.val → Issued (.Unlink (.Record k)) pre :=
  (run_inv hrun).syncing ((run_post hrun h).marker rfl)

/-- A unit is accepted (the machine ready again) only when the landed object
is the unit's intended one. -/
theorem unit_accepted_only_landed {s s' : State} {log : Log} {e : Ev} (hrun : Run s log)
    (hph : s.phase = .Mutating) (h : journal_run.run_step s e = ok (s', .Ready)) :
    ∃ l f, e = .Landed l ∧ s.unit = some f ∧ Rollback.Lands f.intended l :=
  (run_post hrun h).ready rfl hph

end Gripsack.Kernel.JournalRun
