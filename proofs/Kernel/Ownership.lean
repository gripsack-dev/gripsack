import Extracted.GripsackKernel
import Kernel.Recover

/-!
# The per-unit ownership decision (plan/0056 §5 K1)

Theorems over the Aeneas translation of `gripsack_kernel::ownership`
(`Extracted/GripsackKernel.lean`): `decide_unit recorded live intended
consent` for one whole-object destination (tracked copy, template, owned link,
or the removal of one of those).

* `decide_unit_k1` (K1): a decision that writes, replaces or removes the
  destination is issued only if nothing is live, the live object is exactly
  gripsack's last write, or consent was given for a live object.
* `decide_unit_drift_never_written`: without consent, a live object that is not
  gripsack's last write is never written — preserved drift, a drifted owned
  destination and an unrecorded object alike.
* `decide_unit_owned_drift_refused` (FND-1): a recorded destination whose live
  object is no longer the recorded one is refused as drifted, links included.
* `decide_unit_unrecorded_refused`: an unrecorded live object is refused as
  unowned, wherever a link points.
* `decide_unit_identity_only`: the decision depends on identities only through
  equality — renaming identities by any injective map changes nothing, so the
  target of a link (a digest the host computed from it) cannot confer
  ownership.
* `decide_unit_takeover_acquires`: a take-over needs consent, a deploy intent,
  and a live object that is not gripsack's own write.
* `decide_unit_progress`: absence is created; gripsack's own write is kept when
  it is the intent and replaced otherwise.
* `decide_unit_unsupported_refused`: exactly the objects without an identity
  are refused as unsupported.

Every theorem unfolds the shipped helpers itself (no per-helper specs), so a
mutant of a helper is rejected by the claim it breaks, not by a helper lemma.
The exact live object a take-over absorbs is pinned by the host: the journal
re-check refuses the mutation if the destination changed after the decision
(S5; plan/0056 §4).
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel

/-- Decisions that change the destination. -/
def Writes : ownership.Decision → Prop
  | .Create => True
  | .ReplaceOwn => True
  | .TakeOver => True
  | .Satisfied => False
  | .Refuse _ => False

/-- The live object is exactly gripsack's last write. -/
def Ours (r : ownership.Recorded) (l : recover.Live) : Prop :=
  ∃ o, l = .Obj o ∧ r = .Wrote o

open Classical in
/-- `Ident::same` as a value: it always returns, and returns equality. -/
theorem Ident.same_ok (a b : recover.Ident) :
    recover.Ident.same a b = ok (decide (a = b)) := by
  obtain ⟨r, hr, hp⟩ := spec_exists_ok (Ident.same_spec a b)
  rw [hr]
  cases r <;> by_cases h : a = b <;> simp_all

/-- The unfolding every claim below starts from. -/
macro "ownership_cases" : tactic => `(tactic| (
  simp only [ownership.decide_unit, ownership.Recorded.wrote, ownership.acquires,
    ownership.Consent.grants_take_over, ownership.refusal, recover.intends, Ident.same_ok,
    decide_eq_true_eq]
  all_goals (try simp only [bind_tc_ok, bind_ok, decide_eq_true_eq])
  all_goals (try split_ifs)
  all_goals (repeat' split)
  all_goals simp_all [Writes, Ours, WP.spec_ok]))

/-- `Decision::writes` is `Writes`. -/
theorem Decision.writes_spec (d : ownership.Decision) :
    ownership.Decision.writes d ⦃ b => b ↔ Writes d ⦄ := by
  cases d <;> simp [ownership.Decision.writes, Writes]

/-- K1: a decision that writes, replaces or removes the destination is issued
only if nothing is live, the live object is gripsack's last write, or the
user consented to absorbing the live object. -/
theorem decide_unit_k1 (r : ownership.Recorded) (l : recover.Live)
    (i : recover.Intended) (c : ownership.Consent) :
    ownership.decide_unit r l i c ⦃ d =>
      Writes d →
        l = .Absent ∨ Ours r l ∨ (c.take_over = .Granted ∧ ∃ o, l = .Obj o) ⦄ := by
  obtain ⟨t⟩ := c
  cases l <;> cases i <;> cases r <;> cases t <;> ownership_cases

/-- Without consent, a live object that is not gripsack's last write is never
written: preserved drift, a user's replacement of an owned destination, and
an unrecorded object alike. -/
theorem decide_unit_drift_never_written (r : ownership.Recorded) (o : recover.Ident)
    (i : recover.Intended) (c : ownership.Consent)
    (hc : c.take_over = .Withheld) (hr : r ≠ .Wrote o) :
    ownership.decide_unit r (.Obj o) i c ⦃ d => ¬ Writes d ⦄ := by
  obtain ⟨t⟩ := c
  cases i <;> cases r <;> cases t <;> ownership_cases

/-- FND-1: a recorded destination whose live object is no longer the recorded
one — the user replaced gripsack's file or link — is refused as drifted
unless the user consents, whatever the record is. -/
theorem decide_unit_owned_drift_refused (w o : recover.Ident)
    (i : recover.Intended) (c : ownership.Consent)
    (hc : c.take_over = .Withheld) (hd : o ≠ w) (hi : i ≠ .Obj o) :
    ownership.decide_unit (.Wrote w) (.Obj o) i c ⦃ d => d = .Refuse .Drifted ⦄ := by
  obtain ⟨t⟩ := c
  cases i <;> cases t <;> ownership_cases

/-- Nothing records the destination: the live object is refused as unowned
unless the user consents, wherever it points. -/
theorem decide_unit_unrecorded_refused (o : recover.Ident)
    (i : recover.Intended) (c : ownership.Consent)
    (hc : c.take_over = .Withheld) (hi : i ≠ .Obj o) :
    ownership.decide_unit .Unrecorded (.Obj o) i c ⦃ d => d = .Refuse .Unowned ⦄ := by
  obtain ⟨t⟩ := c
  cases i <;> cases t <;> ownership_cases

/-- Renaming identities. -/
def Recorded.map (f : recover.Ident → recover.Ident) : ownership.Recorded → ownership.Recorded
  | .Unrecorded => .Unrecorded
  | .Observed => .Observed
  | .Wrote o => .Wrote (f o)

def Live.map (f : recover.Ident → recover.Ident) : recover.Live → recover.Live
  | .Absent => .Absent
  | .Obj o => .Obj (f o)
  | .Unsupported => .Unsupported

def Intended.map (f : recover.Ident → recover.Ident) : recover.Intended → recover.Intended
  | .Removed => .Removed
  | .Obj o => .Obj (f o)

/-- The decision depends on identities only through equality: for any
injective renaming of identities, the decision is unchanged. A link's target
reaches the kernel only as a digest inside an identity, so where a link
points can matter only by being equal to the recorded or intended object. -/
theorem decide_unit_identity_only (f : recover.Ident → recover.Ident)
    (hf : Function.Injective f) (r : ownership.Recorded) (l : recover.Live)
    (i : recover.Intended) (c : ownership.Consent) :
    ownership.decide_unit (Recorded.map f r) (Live.map f l) (Intended.map f i) c =
      ownership.decide_unit r l i c := by
  obtain ⟨t⟩ := c
  cases l <;> cases i <;> cases r <;> cases t <;>
    simp [Recorded.map, Live.map, Intended.map, ownership.decide_unit,
      ownership.Recorded.wrote, ownership.acquires, ownership.Consent.grants_take_over,
      ownership.refusal, recover.intends, Ident.same_ok, decide_eq_true_eq, hf.eq_iff] <;>
    split_ifs <;> simp_all [hf.eq_iff]

/-- A take-over needs consent and a deploy intent, and never absorbs
gripsack's own write. -/
theorem decide_unit_takeover_acquires (r : ownership.Recorded) (l : recover.Live)
    (i : recover.Intended) (c : ownership.Consent) :
    ownership.decide_unit r l i c ⦃ d =>
      d = .TakeOver → c.take_over = .Granted ∧ i ≠ .Removed ∧ ¬ Ours r l ⦄ := by
  obtain ⟨t⟩ := c
  cases l <;> cases i <;> cases r <;> cases t <;> ownership_cases

open Classical in
/-- gripsack converges on what is its own: an absent destination is created,
and its own last write is kept when it already is the intent and replaced
(or removed) otherwise — whatever the consent. -/
theorem decide_unit_progress (r : ownership.Recorded) (l : recover.Live)
    (i : recover.Intended) (c : ownership.Consent) :
    ownership.decide_unit r l i c ⦃ d =>
      (l = .Absent → i ≠ .Removed → d = .Create) ∧
      (∀ o, l = .Obj o → r = .Wrote o →
        d = (if i = .Obj o then .Satisfied else .ReplaceOwn)) ⦄ := by
  obtain ⟨t⟩ := c
  cases l <;> cases i <;> cases r <;> cases t <;> ownership_cases

/-- Exactly the objects without an identity are refused as unsupported. -/
theorem decide_unit_unsupported_refused (r : ownership.Recorded) (l : recover.Live)
    (i : recover.Intended) (c : ownership.Consent) :
    ownership.decide_unit r l i c ⦃ d => d = .Refuse .Unsupported ↔ l = .Unsupported ⦄ := by
  obtain ⟨t⟩ := c
  cases l <;> cases i <;> cases r <;> cases t <;> ownership_cases

end Gripsack.Kernel
