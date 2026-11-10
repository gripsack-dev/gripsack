import Extracted.GripsackKernel
import Kernel.Selection
import Kernel.Recover

/-!
# The activation machine (plan/0056 F2 step 5, P4 at the kernel level)

Theorems over the Aeneas translation of `gripsack_kernel::activation`
(`Extracted/GripsackKernel.lean`).

The ledger transitions (the former Verus `activation` family, now over the
shipped code):

* `next_attempt_spec`: a non-terminal record gets the attempt that replays it
  — `1` for a pending intent, `n + 1` after an interrupted `Started n` — and a
  terminal record gets none.
* `finish_attempt_spec`: only the matching in-flight attempt settles, as
  `Succeeded` or `Failed` with that attempt.
* `supersede_spec`: a non-terminal record becomes `Superseded`, keeping its
  last attempt.

Over every execution of the machine (`Run`, invariant `Inv`, `step_spec`):

* `hook_after_durable_start`: a hook runs only right after its intent's
  `Started` record was persisted and acknowledged durable — the immediately
  preceding command is that `Persist` — for the intent under decision.
* `hook_selection_current`: a hook runs only when the run read `current` as
  exactly the batch's selection and authorized it, and carries that selection
  as `expect` for the driver's re-check at launch.
* `hook_once_per_run`: no intent runs twice in one run (a failed intent is not
  retried).
* `start_replays`: a `Started` record is written only over a non-terminal
  record, with the attempt that replays it, under the same intent.
* `settle_returned`: a `Succeeded`/`Failed` record is written only for the
  attempt that just returned, as what it returned — a failure is recorded and
  the machine moves on: nothing is rolled back (0001 §3.8).
* `terminal_never_rewritten`, `terminal_never_runs`: `Succeeded`, `Failed`
  and `Superseded` are terminal.
* `superseded_runs_nothing`: when `current` is not the batch's selection, no
  hook ever runs; supersession writes only `Superseded` records.
* `leaves_terminal`: the machine moves past an intent only once its record is
  terminal; `archive_after_every_intent`, `clear_after_archive`: the receipt is
  written after every intent was read, and the pointer cleared only right
  after the receipt.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.Activation

/-! ## The ledger transitions -/

/-- `Succeeded`, `Failed` and `Superseded` are terminal. -/
def Terminal : activation.IntentState → Prop
  | .Pending => False
  | .Started _ => False
  | .Succeeded _ => True
  | .Failed _ => True
  | .Superseded _ => True

/-- Attempt `a` replays record `st`: the first attempt of a pending intent, the
successor of an interrupted one. -/
def Replays : activation.IntentState → activation.AttemptNumber → Prop
  | .Pending, a => a.number.val = 1
  | .Started b, a => a.number.val = b.number.val + 1
  | _, _ => False

/-- The terminal record of attempt `a` that returned `r`. -/
def Settles (a : activation.AttemptNumber) : activation.HookResult → activation.IntentState
  | .Succeeded => .Succeeded a
  | .Failed => .Failed a

/-- `x` supersedes the non-terminal record `st`, keeping its last attempt. -/
def Supersedes : activation.IntentState → activation.IntentState → Prop
  | .Pending, x => x = .Superseded none
  | .Started a, x => x = .Superseded (some a)
  | _, _ => False

theorem Settles.terminal (a : activation.AttemptNumber) (r : activation.HookResult) :
    Terminal (Settles a r) := by
  cases r <;> simp [Settles, Terminal]

theorem Replays.live {st : activation.IntentState} {a : activation.AttemptNumber}
    (h : Replays st a) : ¬ Terminal st := by
  cases st <;> simp_all [Replays, Terminal]

theorem Supersedes.live {st x : activation.IntentState} (h : Supersedes st x) :
    ¬ Terminal st ∧ Terminal x := by
  cases st <;> simp_all [Supersedes, Terminal]

/-- An attempt number below the counter's end has a successor. -/
theorem u64_succ_le {n : U64} (h : ¬ n = core.num.U64.MAX) : n.val + 1 ≤ U64.max := by
  have hne : n.val ≠ core.num.U64.MAX.val := fun he => h (UScalar.eq_of_val_eq he)
  have hb : n.val ≤ U64.max := by scalar_tac
  simp [core.num.U64.MAX, U64.rMax] at hne
  rw [U64.max_eq] at hb ⊢
  omega

theorem AttemptNumber.same_spec (a b : activation.AttemptNumber) :
    activation.AttemptNumber.same a b ⦃ r => r ↔ a = b ⦄ := by
  obtain ⟨a⟩ := a
  obtain ⟨b⟩ := b
  simp [activation.AttemptNumber.same]

theorem terminal_spec (st : activation.IntentState) :
    activation.terminal st ⦃ b => b ↔ Terminal st ⦄ := by
  cases st <;> simp [activation.terminal, Terminal]

/-- The next attempt of a record: the one that replays it, none for a terminal
record, and exhaustion only for an interrupted attempt at the counter's end. -/
theorem next_attempt_spec (st : activation.IntentState) :
    activation.next_attempt st ⦃ d =>
      (∀ a, d = .Start a → Replays st a) ∧ (d = .Terminal ↔ Terminal st) ∧
      (d = .Exhausted → ∃ b, st = .Started b) ⦄ := by
  cases st with
  | Pending => simp [activation.next_attempt, activation.AttemptNumber.first, Replays, Terminal]
  | Started b =>
    obtain ⟨n⟩ := b
    simp only [activation.next_attempt, activation.AttemptNumber.checked_next]
    split_ifs with h
    · simp [Terminal]
    · have := u64_succ_le h
      step as ⟨ i, hi ⟩
      simp [Replays, Terminal, hi]
  | Succeeded _ => simp [activation.next_attempt, Terminal]
  | Failed _ => simp [activation.next_attempt, Terminal]
  | Superseded _ => simp [activation.next_attempt, Terminal]

/-- Only the matching in-flight attempt settles, as what it returned. -/
theorem finish_attempt_spec (st : activation.IntentState) (a : activation.AttemptNumber)
    (r : activation.HookResult) :
    activation.finish_attempt st a r ⦃ o =>
      (∀ x, o = some x → st = .Started a ∧ x = Settles a r) ∧
      (st = .Started a → o = some (Settles a r)) ⦄ := by
  cases st with
  | Started b =>
    simp only [activation.finish_attempt]
    step with AttemptNumber.same_spec as ⟨ h, hh ⟩
    split_ifs with hs
    · cases r <;> simp_all [Settles]
    · simp only [WP.spec_ok]
      refine ⟨by simp, fun he => ?_⟩
      simp only [activation.IntentState.Started.injEq] at he
      exact absurd (hh.mpr he) hs
  | _ => simp [activation.finish_attempt]

/-- Supersession turns a non-terminal record terminal, keeping its last attempt. -/
theorem supersede_spec (st : activation.IntentState) (h : ¬ Terminal st) :
    activation.supersede st ⦃ x => Supersedes st x ⦄ := by
  cases st <;> simp_all [activation.supersede, Supersedes, Terminal]

theorem bound_spec (c : Option selection.SelectionIdentity) (sel : selection.SelectionIdentity) :
    activation.bound c sel ⦃ b => b ↔ c = some sel ⦄ := by
  cases c with
  | none => simp [activation.bound]
  | some c =>
    simp only [activation.bound]
    step with SelectionIdentity.same_spec as ⟨ b, hb ⟩
    simp [hb]

/-! ## What each step helper returns -/

section helpers
variable (s : activation.ActivationState)

theorem refuse_spec' (r : activation.Refusal) :
    activation.refuse s r ⦃ p => p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  simp [activation.refuse]

theorem failed_or_protocol_spec' (e : activation.Event) :
    activation.failed_or_protocol s e ⦃ p =>
      ∃ r, p = ({ s with phase := .Aborted }, .Abort r) ⦄ := by
  unfold activation.failed_or_protocol
  cases e <;> simp [activation.refuse]

/-- Where `proceed` takes the run from `s`: the next intent, or the archive
once the index reached the batch's intent count. -/
def Proceed (p : activation.ActivationState × activation.Command) : Prop :=
  (s.index.val < s.batch.intents.val ∧
    p = ({ s with phase := .ReadingIntent }, .ReadIntent s.index)) ∨
  (s.batch.intents.val ≤ s.index.val ∧ p = ({ s with phase := .Archiving }, .Archive))

theorem proceed_spec : activation.proceed s ⦃ p => Proceed s p ⦄ := by
  unfold activation.proceed Proceed
  split_ifs with h
  · simp only [WP.spec_ok]; left; exact ⟨by scalar_tac, by simp⟩
  · simp only [WP.spec_ok]; right; exact ⟨by scalar_tac, by simp⟩

/-- Where `advance` takes the run: the intent after `s.index`, or a refusal
when the counter would overflow. -/
def Advance (p : activation.ActivationState × activation.Command) : Prop :=
  p = ({ s with phase := .Aborted }, .Abort .Malformed) ∨
  ∃ i : U32, i.val = s.index.val + 1 ∧ Proceed { s with index := i } p

theorem advance_spec : activation.advance s ⦃ p => Advance s p ⦄ := by
  unfold activation.advance Advance
  have h := U32.checked_add_bv_spec s.index 1#u32
  cases ho : U32.checked_add s.index 1#u32 with
  | none => simp [lift, activation.refuse]
  | some i =>
    simp only [ho] at h
    simp only [lift, Std.bind_ok]
    refine WP.spec_mono (proceed_spec { s with index := i }) ?_
    intro p hp
    right
    exact ⟨i, by simpa using h.2.1, hp⟩

theorem on_current_spec (c : Option selection.SelectionIdentity) :
    activation.on_current s c ⦃ p =>
      (c = some s.batch.selection ∧
        p = ({ s with phase := .Authorizing, mode := .Run, current := c, index := 0#u32 },
          .Authorize s.batch.selection)) ∨
      (c ≠ some s.batch.selection ∧
        Proceed { s with mode := .Supersede, current := c, index := 0#u32 } p) ⦄ := by
  unfold activation.on_current
  step with bound_spec as ⟨ b, hb ⟩
  split_ifs with h
  · simp only [WP.spec_ok]; exact Or.inl ⟨hb.mp h, by simp⟩
  · refine WP.spec_mono (proceed_spec _) ?_
    intro p hp
    exact Or.inr ⟨fun he => h (hb.mpr he), hp⟩

theorem on_intent_spec (st : activation.IntentState) :
    activation.on_intent s st ⦃ p =>
      (∃ r, p = ({ s with record := st, phase := .Aborted }, .Abort r)) ∨
      (Terminal st ∧ Advance { s with record := st } p) ∨
      (s.mode = .Run ∧ ∃ a, Replays st a ∧
        p = ({ s with record := .Started a, phase := .Starting }, .Persist s.index (.Started a))) ∨
      (s.mode = .Supersede ∧ ∃ x, Supersedes st x ∧
        p = ({ s with record := x, phase := .Superseding }, .Persist s.index x)) ⦄ := by
  unfold activation.on_intent
  cases hm : s.mode with
  | Run =>
    simp only
    step with next_attempt_spec as ⟨ d, hstart, hterm, hex ⟩
    cases d with
    | Start a =>
      simp only [activation.persist, WP.spec_ok]
      exact Or.inr (Or.inr (Or.inl ⟨a, hstart a rfl, by simp⟩))
    | Terminal =>
      refine WP.spec_mono (advance_spec _) ?_
      intro p hp
      exact Or.inr (Or.inl ⟨hterm.mp rfl, hp⟩)
    | Exhausted =>
      simp only [activation.refuse, WP.spec_ok]
      exact Or.inl ⟨_, rfl⟩
  | Supersede =>
    simp only
    step with terminal_spec as ⟨ b, hb ⟩
    split_ifs with h
    · refine WP.spec_mono (advance_spec _) ?_
      intro p hp
      exact Or.inr (Or.inl ⟨hb.mp h, hp⟩)
    · have hnt : ¬ Terminal st := fun ht => h (hb.mpr ht)
      refine WP.spec_bind (supersede_spec st hnt) ?_
      intro x hx
      simp only [activation.persist, WP.spec_ok]
      exact Or.inr (Or.inr (Or.inr ⟨x, hx, by simp⟩))

theorem on_started_spec :
    activation.on_started s ⦃ p =>
      (∃ r, p = ({ s with phase := .Aborted }, .Abort r)) ∨
      ∃ a, s.record = .Started a ∧
        p = ({ s with phase := .Running }, .RunHook s.index a s.batch.selection) ⦄ := by
  unfold activation.on_started
  cases h : s.record <;> simp [activation.refuse, h]

theorem on_returned_spec (r : activation.HookResult) :
    activation.on_returned s r ⦃ p =>
      (∃ q, p = ({ s with phase := .Aborted }, .Abort q)) ∨
      ∃ a, s.record = .Started a ∧
        p = ({ s with phase := .Settling, record := Settles a r }, .Persist s.index (Settles a r)) ⦄ := by
  unfold activation.on_returned
  cases h : s.record with
  | Started a =>
    simp only
    step with finish_attempt_spec as ⟨ o, ho, ho' ⟩
    subst ho'
    simp [activation.persist]
  | _ => simp [activation.refuse, h]

theorem on_launch_failed_spec (f : recover.Failure) :
    activation.on_launch_failed s f ⦃ p => ∃ q, p = ({ s with phase := .Aborted }, .Abort q) ⦄ := by
  unfold activation.on_launch_failed
  cases f <;> simp [activation.refuse]

theorem finished_spec :
    activation.finished s ⦃ p => ∃ o, p = ({ s with phase := .Finished }, .Finish o) ⦄ := by
  unfold activation.finished
  cases s.mode <;> simp

end helpers

/-! ## The invariant of every run -/

/-- Phases before the ledger walk. -/
def Early (ph : activation.Phase) : Prop :=
  ph = .Start ∨ ph = .ReadingBatch ∨ ph = .ReadingCurrent ∨ ph = .Authorizing

/-- Phases before `current` was read. -/
def Pre (ph : activation.Phase) : Prop :=
  ph = .Start ∨ ph = .ReadingBatch ∨ ph = .ReadingCurrent

/-- Phases in which the intent under decision has been read. -/
def Deciding (ph : activation.Phase) : Prop :=
  ph = .ReadingIntent ∨ ph = .Superseding ∨ ph = .Starting ∨ ph = .Running ∨ ph = .Settling

/-- Phases after the hook of the intent under decision was launched. -/
def Launched (ph : activation.Phase) : Prop :=
  ph = .Running ∨ ph = .Settling ∨ ph = .Aborted

/-- Phases that persist a terminal record. -/
def Closing (ph : activation.Phase) : Prop :=
  ph = .Superseding ∨ ph = .Settling

structure Inv (s : activation.ActivationState) (cs : List activation.Command) : Prop where
  /-- Before `current` was read, the machine is at its initial mode and index. -/
  pre : Pre s.phase → s.mode = .Supersede ∧ s.index = 0#u32
  /-- Before the ledger walk, no hook ran. -/
  early : Early s.phase → ∀ i a x, activation.Command.RunHook i a x ∉ cs
  /-- Every hook ran under a bound, authorized run, for an intent before the
  one under decision or for it after its launch. -/
  launched : ∀ i a x, activation.Command.RunHook i a x ∈ cs →
    s.mode = .Run ∧ x = s.batch.selection ∧
    (i.val < s.index.val ∨ (i = s.index ∧ Launched s.phase))
  /-- The run mode comes only from a bound `current`, after authorization. -/
  bound : s.mode = .Run →
    s.current = some s.batch.selection ∧ activation.Command.Authorize s.batch.selection ∈ cs
  /-- Every intent before the one under decision was read. -/
  read : ∀ k : U32, k.val < s.index.val → activation.Command.ReadIntent k ∈ cs
  /-- The intent under decision was read. -/
  reading : Deciding s.phase → activation.Command.ReadIntent s.index ∈ cs
  /-- While starting, the `Started` record was the last command. -/
  starting : s.phase = .Starting → s.mode = .Run ∧ ∃ a, s.record = .Started a ∧
    cs.getLast? = some (.Persist s.index (.Started a))
  /-- While a hook runs, its record is `Started`. -/
  running : s.phase = .Running → s.mode = .Run ∧ ∃ a, s.record = .Started a
  /-- A closing record is terminal. -/
  closing : Closing s.phase → Terminal s.record
  /-- While archiving, the receipt was the last command. -/
  archiving : s.phase = .Archiving → cs.getLast? = some .Archive

/-- The executions of the machine: from `ActivationState::new`, any sequence of
events the step accepts, with the commands it issued, in order. -/
inductive Run : activation.ActivationState → List activation.Command → Prop where
  | start {s} : activation.ActivationState.new = ok s → Run s []
  | step {s cs e s' c} : Run s cs → activation.activation_step s e = ok (s', c) →
      Run s' (cs ++ [c])

/-- What one step guarantees, given the invariant before it. -/
structure StepPost (s : activation.ActivationState) (e : activation.Event)
    (cs : List activation.Command) (p : activation.ActivationState × activation.Command) : Prop where
  inv : Inv p.1 (cs ++ [p.2])
  hook : ∀ i a x, p.2 = .RunHook i a x →
    s.phase = .Starting ∧ e = .Done ∧ i = s.index ∧ s.record = .Started a ∧
    cs.getLast? = some (.Persist i (.Started a)) ∧ x = s.batch.selection ∧ s.mode = .Run ∧
    s.current = some x ∧ activation.Command.Authorize x ∈ cs ∧
    ∀ b y, activation.Command.RunHook i b y ∉ cs
  persist : ∀ i x, p.2 = .Persist i x → i = s.index ∧
    ((s.phase = .ReadingIntent ∧ ∃ st, e = .IntentRead st ∧ ¬ Terminal st ∧
        ((s.mode = .Run ∧ ∃ a, x = .Started a ∧ Replays st a) ∨
         (s.mode = .Supersede ∧ Supersedes st x))) ∨
     (s.phase = .Running ∧ ∃ a r, s.record = .Started a ∧ e = .Returned r ∧ x = Settles a r))
  leave : (p.2 = .Archive ∨ ∃ j, p.2 = .ReadIntent j) → Closing s.phase ∨ s.phase = .ReadingIntent →
    Terminal p.1.record
  archive : p.2 = .Archive → ∀ k : U32, k.val < p.1.batch.intents.val →
    activation.Command.ReadIntent k ∈ cs
  clear : p.2 = .Clear → s.phase = .Archiving ∧ e = .Done ∧ cs.getLast? = some .Archive

/-- A command none of `StepPost`'s clauses constrain. -/
def Quiet (c : activation.Command) : Prop :=
  (∀ i a x, c ≠ .RunHook i a x) ∧ (∀ i x, c ≠ .Persist i x) ∧ c ≠ .Archive ∧
  (∀ j, c ≠ .ReadIntent j) ∧ c ≠ .Clear

theorem StepPost.of_quiet {s e cs p} (hi : Inv p.1 (cs ++ [p.2])) (hq : Quiet p.2) :
    StepPost s e cs p where
  inv := hi
  hook := fun i a x h => absurd h (hq.1 i a x)
  persist := fun i x h => absurd h (hq.2.1 i x)
  leave := fun h => by
    rcases h with h | ⟨j, h⟩
    · exact absurd h hq.2.2.1
    · exact absurd h (hq.2.2.2.1 j)
  archive := fun h => absurd h hq.2.2.1
  clear := fun h => absurd h hq.2.2.2.2

theorem mem_snoc {α : Type} {x y : α} {cs : List α} (h : x ∈ cs ++ [y]) : x ∈ cs ∨ x = y := by
  simpa using h

/-- An abort keeps everything but the phase. -/
theorem post_abort {s : activation.ActivationState} {e cs} (h : Inv s cs)
    (r : activation.Refusal) (t : activation.ActivationState)
    (hb : t.batch = s.batch) (hm : t.mode = s.mode) (hc : t.current = s.current)
    (hi : t.index = s.index) :
    StepPost s e cs ({ t with phase := .Aborted }, .Abort r) := by
  refine StepPost.of_quiet ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩ (by simp [Quiet])
  · simp [Pre]
  · simp [Early]
  · intro i a x hx
    rcases mem_snoc hx with hx | hx
    · obtain ⟨h1, h2, h3⟩ := h.launched i a x hx
      refine ⟨hm ▸ h1, hb ▸ h2, ?_⟩
      rcases h3 with h3 | ⟨rfl, -⟩
      · exact Or.inl (hi ▸ h3)
      · exact Or.inr ⟨hi.symm, by simp [Launched]⟩
    · simp at hx
  · intro hrun
    obtain ⟨h1, h2⟩ := h.bound (hm ▸ hrun)
    exact ⟨hc ▸ hb ▸ h1, hb ▸ List.mem_append_left _ h2⟩
  · intro k hk; exact List.mem_append_left _ (h.read k (hi ▸ hk))
  · simp [Deciding]
  · simp
  · simp
  · simp [Closing]
  · simp

/-- Entering `proceed` from state `t`. -/
theorem post_proceed {s : activation.ActivationState} {e cs} {t : activation.ActivationState}
    {p : activation.ActivationState × activation.Command} (hp : Proceed t p)
    (hlaunch : ∀ i a x, activation.Command.RunHook i a x ∈ cs →
      t.mode = .Run ∧ x = t.batch.selection ∧ i.val < t.index.val)
    (hbound : t.mode = .Run →
      t.current = some t.batch.selection ∧ activation.Command.Authorize t.batch.selection ∈ cs)
    (hread : ∀ k : U32, k.val < t.index.val → activation.Command.ReadIntent k ∈ cs)
    (hleave : Closing s.phase ∨ s.phase = .ReadingIntent → Terminal t.record) :
    StepPost s e cs p := by
  have launched : ∀ (ph : activation.Phase) (c : activation.Command), (∀ i a x, c ≠ .RunHook i a x) →
      ∀ i a x, activation.Command.RunHook i a x ∈ cs ++ [c] →
        t.mode = .Run ∧ x = t.batch.selection ∧
        (i.val < t.index.val ∨ (i = t.index ∧ Launched ph)) := by
    intro ph c hc i a x hx
    rcases mem_snoc hx with hx | hx
    · obtain ⟨h1, h2, h3⟩ := hlaunch i a x hx
      exact ⟨h1, h2, Or.inl h3⟩
    · exact absurd hx.symm (hc i a x)
  have bound : ∀ c : activation.Command, t.mode = .Run →
      t.current = some t.batch.selection ∧
        activation.Command.Authorize t.batch.selection ∈ cs ++ [c] := by
    intro c hm
    obtain ⟨h1, h2⟩ := hbound hm
    exact ⟨h1, List.mem_append_left _ h2⟩
  have read : ∀ c : activation.Command, ∀ k : U32, k.val < t.index.val →
      activation.Command.ReadIntent k ∈ cs ++ [c] :=
    fun c k hk => List.mem_append_left _ (hread k hk)
  rcases hp with ⟨hlt, rfl⟩ | ⟨hle, rfl⟩
  · refine ⟨⟨by simp [Pre], by simp [Early], launched _ _ (by simp), bound _, read _, by simp,
      by simp, by simp, by simp [Closing], by simp⟩, by simp, by simp, ?_, by simp, by simp⟩
    intro _ hl
    exact hleave hl
  · refine ⟨⟨by simp [Pre], by simp [Early], launched _ _ (by simp), bound _, read _,
      by simp [Deciding], by simp, by simp, by simp [Closing], by simp⟩, by simp, by simp, ?_, ?_,
      by simp⟩
    · intro _ hl
      exact hleave hl
    · intro _ k hk
      exact hread k (by simp only at hk ⊢; omega)

/-- Moving past the intent under decision (its record now terminal). -/
theorem post_advance {s : activation.ActivationState} {e cs} (h : Inv s cs)
    {t : activation.ActivationState} {p : activation.ActivationState × activation.Command}
    (ha : Advance t p) (hb : t.batch = s.batch) (hm : t.mode = s.mode)
    (hc : t.current = s.current) (hi : t.index = s.index) (hdec : Deciding s.phase)
    (hterm : Terminal t.record) : StepPost s e cs p := by
  rcases ha with rfl | ⟨i, hi', hp⟩
  · exact post_abort h _ t hb hm hc hi
  · refine post_proceed hp ?_ ?_ ?_ (fun _ => hterm)
    · intro j a x hx
      obtain ⟨h1, h2, h3⟩ := h.launched j a x hx
      refine ⟨hm ▸ h1, hb ▸ h2, ?_⟩
      simp only at hi' ⊢
      rw [hi] at hi'
      rcases h3 with h3 | ⟨rfl, -⟩
      · omega
      · omega
    · intro hrun
      obtain ⟨h1, h2⟩ := h.bound (hm ▸ hrun)
      exact ⟨hc ▸ hb ▸ h1, hb ▸ h2⟩
    · intro k hk
      simp only at hk hi'
      rw [hi] at hi'
      by_cases hlt : k.val < s.index.val
      · exact h.read k hlt
      · have : k = s.index := UScalar.eq_of_val_eq (by omega)
        subst this
        exact h.reading hdec

theorem step_spec (s : activation.ActivationState) (e : activation.Event)
    (cs : List activation.Command) (h : Inv s cs) :
    activation.activation_step s e ⦃ p => StepPost s e cs p ⦄ := by
  rcases s with ⟨phase, mode, batch, current, index, record⟩
  unfold activation.activation_step
  cases phase <;> cases e
  all_goals try first
    | (with_reducible exact WP.spec_mono (failed_or_protocol_spec' _ _) (by rintro p ⟨r, rfl⟩; exact post_abort h r _ rfl rfl rfl rfl))
    | (with_reducible exact WP.spec_mono (refuse_spec' _ _) (by rintro p rfl; exact post_abort h _ _ rfl rfl rfl rfl))
  case Start.Start =>
    simp only [WP.spec_ok]
    have hpre := h.pre (by simp [Pre])
    refine StepPost.of_quiet ⟨fun _ => hpre, fun _ => ?_, ?_, ?_, ?_,
      by simp [Deciding], by simp, by simp, by simp [Closing], by simp⟩ (by simp [Quiet])
    · intro i a x hx
      rcases mem_snoc hx with hx | hx
      · exact h.early (by simp [Early]) i a x hx
      · simp at hx
    · intro i a x hx
      rcases mem_snoc hx with hx | hx
      · exact absurd hx (h.early (by simp [Early]) i a x)
      · simp at hx
    · intro hm; simp_all
    · intro k hk; simp_all
  case ReadingBatch.NoBatch =>
    simp only [WP.spec_ok]
    refine StepPost.of_quiet ⟨by simp [Pre], by simp [Early], ?_, ?_, ?_,
      by simp [Deciding], by simp, by simp, by simp [Closing], by simp⟩ (by simp [Quiet])
    · intro i a x hx
      rcases mem_snoc hx with hx | hx
      · exact absurd hx (h.early (by simp [Early]) i a x)
      · simp at hx
    · intro hm; have := (h.pre (by simp [Pre])).1; simp_all
    · intro k hk; have := (h.pre (by simp [Pre])).2; simp_all
  case ReadingBatch.BatchRead b =>
    simp only [WP.spec_ok]
    have hpre := h.pre (by simp [Pre])
    refine StepPost.of_quiet ⟨fun _ => hpre, fun _ => ?_, ?_, ?_, ?_,
      by simp [Deciding], by simp, by simp, by simp [Closing], by simp⟩ (by simp [Quiet])
    · intro i a x hx
      rcases mem_snoc hx with hx | hx
      · exact h.early (by simp [Early]) i a x hx
      · simp at hx
    · intro i a x hx
      rcases mem_snoc hx with hx | hx
      · exact absurd hx (h.early (by simp [Early]) i a x)
      · simp at hx
    · intro hm; simp_all
    · intro k hk; simp_all
  case ReadingCurrent.CurrentRead c =>
    have hpre := h.pre (by simp [Pre])
    have hnone : ∀ i a x, activation.Command.RunHook i a x ∉ cs := h.early (by simp [Early])
    refine WP.spec_mono (on_current_spec _ c) ?_
    rintro p (⟨hc, rfl⟩ | ⟨hc, hp⟩)
    · refine StepPost.of_quiet ⟨by simp [Pre], fun _ => ?_, ?_, ?_, by simp,
        by simp [Deciding], by simp, by simp, by simp [Closing], by simp⟩ (by simp [Quiet])
      · intro i a x hx
        rcases mem_snoc hx with hx | hx
        · exact hnone i a x hx
        · simp at hx
      · intro i a x hx
        rcases mem_snoc hx with hx | hx
        · exact absurd hx (hnone i a x)
        · simp at hx
      · intro _; exact ⟨hc, by simp⟩
    · refine post_proceed hp ?_ (by simp) (by simp) (by simp [Closing])
      intro i a x hx
      exact absurd hx (hnone i a x)
  case Authorizing.Done =>
    have hnone : ∀ i a x, activation.Command.RunHook i a x ∉ cs := h.early (by simp [Early])
    refine WP.spec_mono (proceed_spec _) ?_
    intro p hp
    refine post_proceed hp (fun i a x hx => absurd hx (hnone i a x)) h.bound
      h.read (by simp [Closing])
  case ReadingIntent.IntentRead st =>
    refine WP.spec_mono (on_intent_spec _ st) ?_
    rintro p (⟨r, rfl⟩ | ⟨ht, ha⟩ | ⟨hm, a, hr, rfl⟩ | ⟨hm, x, hx, rfl⟩)
    · exact post_abort h r ⟨.ReadingIntent, mode, batch, current, index, st⟩ rfl rfl rfl rfl
    · exact post_advance h ha rfl rfl rfl rfl (by simp [Deciding]) ht
    · have hreading := h.reading (by simp [Deciding])
      refine ⟨⟨by simp [Pre], by simp [Early], ?_, ?_, ?_, fun _ => List.mem_append_left _ hreading,
        fun _ => ⟨hm, a, rfl, by simp⟩, by simp, by simp [Closing], by simp⟩,
        by simp, ?_, by simp, by simp, by simp⟩
      · intro i a' x hx
        rcases mem_snoc hx with hx | hx
        · obtain ⟨h1, h2, h3⟩ := h.launched i a' x hx
          refine ⟨h1, h2, Or.inl ?_⟩
          rcases h3 with h3 | ⟨-, h3⟩
          · exact h3
          · simp [Launched] at h3
        · simp at hx
      · intro hrun
        obtain ⟨h1, h2⟩ := h.bound hrun
        exact ⟨h1, List.mem_append_left _ h2⟩
      · intro k hk; exact List.mem_append_left _ (h.read k hk)
      · intro i x hp
        simp only [activation.Command.Persist.injEq] at hp
        obtain ⟨rfl, rfl⟩ := hp
        exact ⟨rfl, Or.inl ⟨rfl, st, rfl, hr.live, Or.inl ⟨hm, a, rfl, hr⟩⟩⟩
    · have hreading := h.reading (by simp [Deciding])
      refine ⟨⟨by simp [Pre], by simp [Early], ?_, ?_, ?_, fun _ => List.mem_append_left _ hreading,
        by simp, by simp, fun _ => hx.live.2, by simp⟩,
        by simp, ?_, by simp, by simp, by simp⟩
      · intro i a' x' hx'
        rcases mem_snoc hx' with hx' | hx'
        · obtain ⟨h1, h2, h3⟩ := h.launched i a' x' hx'
          refine ⟨h1, h2, Or.inl ?_⟩
          rcases h3 with h3 | ⟨-, h3⟩
          · exact h3
          · simp [Launched] at h3
        · simp at hx'
      · intro hrun
        obtain ⟨h1, h2⟩ := h.bound hrun
        exact ⟨h1, List.mem_append_left _ h2⟩
      · intro k hk; exact List.mem_append_left _ (h.read k hk)
      · intro i x' hp
        simp only [activation.Command.Persist.injEq] at hp
        obtain ⟨rfl, rfl⟩ := hp
        exact ⟨rfl, Or.inl ⟨rfl, st, rfl, hx.live.1, Or.inr ⟨hm, hx⟩⟩⟩
  case Superseding.Done =>
    refine WP.spec_mono (advance_spec _) ?_
    intro p hp
    exact post_advance h hp rfl rfl rfl rfl (by simp [Deciding]) (h.closing (by simp [Closing]))
  case Settling.Done =>
    refine WP.spec_mono (advance_spec _) ?_
    intro p hp
    exact post_advance h hp rfl rfl rfl rfl (by simp [Deciding]) (h.closing (by simp [Closing]))
  case Starting.Done =>
    obtain ⟨hm, a0, hrec, hlast⟩ := h.starting rfl
    refine WP.spec_mono (on_started_spec _) ?_
    rintro p (⟨r, rfl⟩ | ⟨a, ha, rfl⟩)
    · exact post_abort h r _ rfl rfl rfl rfl
    · simp only at ha hrec hm
      rw [hrec] at ha
      cases ha
      have hreading := h.reading (by simp [Deciding])
      obtain ⟨hcur, hauth⟩ := h.bound hm
      have hfirst : ∀ b y, activation.Command.RunHook index b y ∉ cs := by
        intro b y hx
        obtain ⟨-, -, h3⟩ := h.launched index b y hx
        simp only at h3
        rcases h3 with h3 | ⟨-, h3⟩
        · omega
        · simp [Launched] at h3
      refine ⟨⟨by simp [Pre], by simp [Early], ?_, ?_, ?_, fun _ => List.mem_append_left _ hreading,
        by simp, fun _ => ⟨hm, a0, hrec⟩, by simp [Closing], by simp⟩, ?_, by simp, by simp, by simp,
        by simp⟩
      · intro i a' x hx
        rcases mem_snoc hx with hx | hx
        · obtain ⟨h1, h2, h3⟩ := h.launched i a' x hx
          refine ⟨h1, h2, ?_⟩
          rcases h3 with h3 | ⟨-, h3⟩
          · exact Or.inl h3
          · simp [Launched] at h3
        · simp only [activation.Command.RunHook.injEq] at hx
          obtain ⟨rfl, -, rfl⟩ := hx
          exact ⟨hm, rfl, Or.inr ⟨rfl, by simp [Launched]⟩⟩
      · intro hrun
        exact ⟨hcur, List.mem_append_left _ hauth⟩
      · intro k hk; exact List.mem_append_left _ (h.read k hk)
      · intro i a' x hx
        simp only [activation.Command.RunHook.injEq] at hx
        obtain ⟨rfl, rfl, rfl⟩ := hx
        exact ⟨rfl, rfl, rfl, hrec, hlast, rfl, hm, hcur, hauth, hfirst⟩
  case Running.Returned r =>
    obtain ⟨hm, a0, hrec⟩ := h.running rfl
    refine WP.spec_mono (on_returned_spec _ r) ?_
    rintro p (⟨q, rfl⟩ | ⟨a, ha, rfl⟩)
    · exact post_abort h q _ rfl rfl rfl rfl
    · have hreading := h.reading (by simp [Deciding])
      refine ⟨⟨by simp [Pre], by simp [Early], ?_, ?_, ?_, fun _ => List.mem_append_left _ hreading,
        by simp, by simp, fun _ => Settles.terminal a r, by simp⟩, by simp, ?_, by simp, by simp,
        by simp⟩
      · intro i a' x hx
        rcases mem_snoc hx with hx | hx
        · obtain ⟨h1, h2, h3⟩ := h.launched i a' x hx
          refine ⟨h1, h2, ?_⟩
          rcases h3 with h3 | ⟨h3, -⟩
          · exact Or.inl h3
          · exact Or.inr ⟨h3, by simp [Launched]⟩
        · simp at hx
      · intro hrun
        obtain ⟨h1, h2⟩ := h.bound hrun
        exact ⟨h1, List.mem_append_left _ h2⟩
      · intro k hk; exact List.mem_append_left _ (h.read k hk)
      · intro i x hp
        simp only [activation.Command.Persist.injEq] at hp
        obtain ⟨rfl, rfl⟩ := hp
        exact ⟨rfl, Or.inr ⟨rfl, a, r, ha, rfl, rfl⟩⟩
  case Running.Failed f =>
    exact WP.spec_mono (on_launch_failed_spec _ f)
      (by rintro p ⟨q, rfl⟩; exact post_abort h q _ rfl rfl rfl rfl)
  case Archiving.Done =>
    simp only [WP.spec_ok]
    refine ⟨⟨by simp [Pre], by simp [Early], ?_, ?_, ?_, by simp [Deciding], by simp, by simp,
      by simp [Closing], by simp⟩, by simp, by simp, by simp, by simp, ?_⟩
    · intro i a x hx
      rcases mem_snoc hx with hx | hx
      · obtain ⟨h1, h2, h3⟩ := h.launched i a x hx
        refine ⟨h1, h2, ?_⟩
        rcases h3 with h3 | ⟨-, h3⟩
        · exact Or.inl h3
        · simp [Launched] at h3
      · simp at hx
    · intro hrun
      obtain ⟨h1, h2⟩ := h.bound hrun
      exact ⟨h1, List.mem_append_left _ h2⟩
    · intro k hk; exact List.mem_append_left _ (h.read k hk)
    · intro _
      exact ⟨rfl, rfl, h.archiving rfl⟩
  case Clearing.Done =>
    refine WP.spec_mono (finished_spec _) ?_
    rintro p ⟨o, rfl⟩
    refine StepPost.of_quiet ⟨by simp [Pre], by simp [Early], ?_, ?_, ?_, by simp [Deciding],
      by simp, by simp, by simp [Closing], by simp⟩ (by simp [Quiet])
    · intro i a x hx
      rcases mem_snoc hx with hx | hx
      · obtain ⟨h1, h2, h3⟩ := h.launched i a x hx
        refine ⟨h1, h2, ?_⟩
        rcases h3 with h3 | ⟨-, h3⟩
        · exact Or.inl h3
        · simp [Launched] at h3
      · simp at hx
    · intro hrun
      obtain ⟨h1, h2⟩ := h.bound hrun
      exact ⟨h1, List.mem_append_left _ h2⟩
    · intro k hk; exact List.mem_append_left _ (h.read k hk)

/-! ## The theorems -/

/-- Every execution keeps the invariant. -/
theorem run_inv {s : activation.ActivationState} {cs : List activation.Command} (h : Run s cs) :
    Inv s cs := by
  induction h with
  | start hs =>
    simp only [activation.ActivationState.new, generation.GenerationId.new, Std.bind_ok,
      ok.injEq] at hs
    subst hs
    exact ⟨fun _ => ⟨rfl, rfl⟩, fun _ _ _ _ h => by simp at h, fun _ _ _ h => by simp at h,
      fun h => by simp at h, fun k hk => by simp at hk, fun h => by simp [Deciding] at h,
      fun h => by simp at h, fun h => by simp at h, fun h => by simp [Closing] at h,
      fun h => by simp at h⟩
  | step _ hstep ih => exact (spec_eq_ok (step_spec _ _ _ ih) hstep).inv

theorem run_post {s s' : activation.ActivationState} {cs : List activation.Command}
    {e : activation.Event} {c : activation.Command} (hrun : Run s cs)
    (h : activation.activation_step s e = ok (s', c)) : StepPost s e cs (s', c) :=
  spec_eq_ok (step_spec _ _ _ (run_inv hrun)) h

/-- P4: in every execution, a hook runs only in answer to the durable
acknowledgement (`Done`) of its intent's `Started` record — the immediately
preceding command is that `Persist` — for the intent under decision, whose
record is `Started` with the launched attempt. -/
theorem hook_after_durable_start {s s' : activation.ActivationState}
    {cs : List activation.Command} {e : activation.Event} {i : U32}
    {a : activation.AttemptNumber} {x : selection.SelectionIdentity}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .RunHook i a x)) :
    s.phase = .Starting ∧ e = .Done ∧ i = s.index ∧ s.record = .Started a ∧
      cs.getLast? = some (.Persist i (.Started a)) := by
  obtain ⟨h1, h2, h3, h4, h5, -⟩ := (run_post hrun h).hook i a x rfl
  exact ⟨h1, h2, h3, h4, h5⟩

/-- P4: in every execution, a hook runs only when the run read `current` as
exactly the batch's selection (transaction bytes included) and authorized
that selection, and the launch carries it as `expect` for the driver's
re-check immediately before the hook starts. -/
theorem hook_selection_current {s s' : activation.ActivationState}
    {cs : List activation.Command} {e : activation.Event} {i : U32}
    {a : activation.AttemptNumber} {x : selection.SelectionIdentity}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .RunHook i a x)) :
    x = s.batch.selection ∧ s.mode = .Run ∧ s.current = some x ∧
      activation.Command.Authorize x ∈ cs := by
  obtain ⟨-, -, -, -, -, h6, h7, h8, h9, -⟩ := (run_post hrun h).hook i a x rfl
  exact ⟨h6, h7, h8, h9⟩

/-- Not at-least-once: in every execution, an intent's hook runs at most once
— a failed attempt is never retried within the run. -/
theorem hook_once_per_run {s s' : activation.ActivationState}
    {cs : List activation.Command} {e : activation.Event} {i : U32}
    {a : activation.AttemptNumber} {x : selection.SelectionIdentity}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .RunHook i a x)) :
    ∀ b y, activation.Command.RunHook i b y ∉ cs :=
  ((run_post hrun h).hook i a x rfl).2.2.2.2.2.2.2.2.2

/-- P4: in every execution, a `Started` record is written only for the intent
under decision, over its non-terminal record as just read, with the attempt
that replays it: `1` for a pending intent, `n + 1` after an interrupted
`Started n` — the same intent, never a fresh identity. -/
theorem start_replays {s s' : activation.ActivationState} {cs : List activation.Command}
    {e : activation.Event} {i : U32} {a : activation.AttemptNumber}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .Persist i (.Started a))) :
    i = s.index ∧ s.phase = .ReadingIntent ∧ s.mode = .Run ∧
      ∃ st, e = .IntentRead st ∧ Replays st a := by
  obtain ⟨hi, hp⟩ := (run_post hrun h).persist i _ rfl
  rcases hp with ⟨hph, st, he, -, ⟨hm, a', ha, hr⟩ | ⟨-, hs⟩⟩ | ⟨-, a', r, -, -, hx⟩
  · cases ha
    exact ⟨hi, hph, hm, st, he, hr⟩
  · cases st <;> simp [Supersedes] at hs
  · cases r <;> simp [Settles] at hx

/-- Hooks are never auto-rolled back (0001 §3.8): in every execution, a
`Succeeded`/`Failed` record is written only in answer to the attempt that just
returned, as what it returned. A failure is recorded and the machine moves on
— its commands contain no flip and no restore at all. -/
theorem settle_returned {s s' : activation.ActivationState} {cs : List activation.Command}
    {e : activation.Event} {i : U32} {x : activation.IntentState} {a : activation.AttemptNumber}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .Persist i x))
    (hx : x = .Succeeded a ∨ x = .Failed a) :
    i = s.index ∧ s.phase = .Running ∧ s.record = .Started a ∧
      ∃ r, e = .Returned r ∧ x = Settles a r := by
  obtain ⟨hi, hp⟩ := (run_post hrun h).persist i x rfl
  rcases hp with ⟨-, st, -, -, ⟨-, a', ha, -⟩ | ⟨-, hs⟩⟩ | ⟨hph, a', r, hrec, he, hset⟩
  · rcases hx with rfl | rfl <;> simp at ha
  · rcases hx with rfl | rfl <;> cases st <;> simp [Supersedes] at hs
  · rcases hx with rfl | rfl <;> cases r <;> simp [Settles] at hset <;> subst hset <;>
      exact ⟨hi, hph, hrec, _, he, rfl⟩

/-- `Succeeded`, `Failed` and `Superseded` are terminal: in every execution, a
record is written only over a non-terminal one — the record just read, or the
`Started` record of the hook that just returned. -/
theorem terminal_never_rewritten {s s' : activation.ActivationState}
    {cs : List activation.Command} {e : activation.Event} {i : U32} {x : activation.IntentState}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .Persist i x)) :
    i = s.index ∧ ((∃ st, e = .IntentRead st ∧ ¬ Terminal st) ∨
      (s.phase = .Running ∧ ∃ a, s.record = .Started a)) := by
  obtain ⟨hi, hp⟩ := (run_post hrun h).persist i x rfl
  rcases hp with ⟨-, st, he, ht, -⟩ | ⟨hph, a, -, hrec, -, -⟩
  · exact ⟨hi, Or.inl ⟨st, he, ht⟩⟩
  · exact ⟨hi, Or.inr ⟨hph, a, hrec⟩⟩

/-- A terminal record never runs: in every execution, reading a terminal
record persists nothing and launches nothing (the machine moves on, or stops). -/
theorem terminal_never_runs {s s' : activation.ActivationState} {cs : List activation.Command}
    {st : activation.IntentState} {c : activation.Command}
    (hrun : Run s cs) (ht : Terminal st)
    (h : activation.activation_step s (.IntentRead st) = ok (s', c)) :
    (∀ i x, c ≠ .Persist i x) ∧ (∀ i a x, c ≠ .RunHook i a x) := by
  have hp := run_post hrun h
  refine ⟨fun i x hc => ?_, fun i a x hc => ?_⟩
  · subst hc
    obtain ⟨-, hp⟩ := hp.persist i x rfl
    rcases hp with ⟨-, st', he, ht', -⟩ | ⟨-, a, r, -, he, -⟩
    · cases he; exact ht' ht
    · cases he
  · subst hc
    obtain ⟨-, he, -⟩ := hp.hook i a x rfl
    cases he

/-- Superseded intents run zero times: in every execution in which `current`
was not read as the batch's selection, no hook has run. -/
theorem superseded_runs_nothing {s : activation.ActivationState} {cs : List activation.Command}
    (hrun : Run s cs) (hc : s.current ≠ some s.batch.selection) :
    ∀ i a x, activation.Command.RunHook i a x ∉ cs := by
  intro i a x hx
  have hi := run_inv hrun
  obtain ⟨hm, -, -⟩ := hi.launched i a x hx
  exact hc (hi.bound hm).1

/-- Supersession writes only `Superseded`: in every execution, a record
persisted while the batch is superseded supersedes the non-terminal record
just read, keeping its last attempt. -/
theorem supersede_writes_superseded {s s' : activation.ActivationState}
    {cs : List activation.Command} {e : activation.Event} {i : U32} {x : activation.IntentState}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .Persist i x))
    (hm : s.mode = .Supersede) :
    ∃ st, e = .IntentRead st ∧ Supersedes st x := by
  obtain ⟨-, hp⟩ := (run_post hrun h).persist i x rfl
  rcases hp with ⟨-, st, he, -, ⟨hm', -⟩ | ⟨-, hs⟩⟩ | ⟨hph, -⟩
  · rw [hm] at hm'; cases hm'
  · exact ⟨st, he, hs⟩
  · have := ((run_inv hrun).running hph).1
    rw [hm] at this; cases this

/-- In every execution, the machine moves past an intent (to the next one, or
to the archive) only once that intent's record is terminal. -/
theorem leaves_terminal {s s' : activation.ActivationState} {cs : List activation.Command}
    {e : activation.Event} {c : activation.Command}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', c))
    (hc : c = .Archive ∨ ∃ j, c = .ReadIntent j)
    (hph : s.phase = .ReadingIntent ∨ s.phase = .Superseding ∨ s.phase = .Settling) :
    Terminal s'.record := by
  refine (run_post hrun h).leave hc ?_
  rcases hph with hph | hph | hph
  · exact Or.inr hph
  · exact Or.inl (Or.inl hph)
  · exact Or.inl (Or.inr hph)

/-- In every execution, the receipt is written only after every intent of the
batch was read (and, by `leaves_terminal`, settled). -/
theorem archive_after_every_intent {s s' : activation.ActivationState}
    {cs : List activation.Command} {e : activation.Event}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .Archive)) :
    ∀ k : U32, k.val < s'.batch.intents.val → activation.Command.ReadIntent k ∈ cs :=
  (run_post hrun h).archive rfl

/-- In every execution, the pending pointer is cleared only in answer to the
durable receipt, which is the immediately preceding command. -/
theorem clear_after_archive {s s' : activation.ActivationState}
    {cs : List activation.Command} {e : activation.Event}
    (hrun : Run s cs) (h : activation.activation_step s e = ok (s', .Clear)) :
    s.phase = .Archiving ∧ e = .Done ∧ cs.getLast? = some .Archive :=
  (run_post hrun h).clear rfl

end Gripsack.Kernel.Activation
