import Extracted.GripsackKernel
import Kernel.Recover

/-!
# The directory sub-protocol (plan/0056 F2 step 9)

Specifications of `gripsack_kernel::directory`, the boundary the apply and
rollback machines share with the host's directory domain: the stage order
items must arrive in, which actions the host executes, and membership in a
list of destinations (the domain's holds, rollback's claims).

What the machines' theorems assume about a commanded step is premise D,
stated in `crates/gripsack-kernel/src/directory.rs` and listed with S1–S7:
a step changes only the namespace of the receipt it names, journals before
its first change, and is recovered only through the recover machine's
delegated hooks. Nothing here proves D; it is the sub-protocol's contract.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.Directory

/-- A stage's position in a run. -/
def Rank : directory.Stage → Nat
  | .Opening => 0
  | .Clearing => 1
  | .Shaping => 2
  | .Holding => 3
  | .Units => 4
  | .Releases => 5
  | .Closing => 6

/-- The stage an action belongs to. -/
def StageOf : directory.Action → directory.Stage
  | .Bind => .Opening
  | .Birth => .Opening
  | .Plan => .Opening
  | .Evolve => .Shaping
  | .Shape => .Shaping
  | .Exchange => .Shaping
  | .Reactivate => .Shaping
  | .Hold => .Holding
  | .Finish => .Closing
  | .Retire => .Closing

/-- The actions the host executes: all but a hold. -/
def Commands : directory.Action → Prop
  | .Hold => False
  | _ => True

theorem ordered_spec (a b : directory.Stage) :
    directory.ordered a b ⦃ r => r ↔ Rank a ≤ Rank b ⦄ := by
  cases a <;> cases b <;> simp [directory.ordered, directory.rank, Rank]

theorem stage_ok (a : directory.Action) : directory.stage a = ok (StageOf a) := by
  cases a <;> rfl

theorem commands_spec (a : directory.Action) :
    directory.commands a ⦃ r => r ↔ Commands a ⦄ := by
  cases a <;> simp [directory.commands, Commands]

theorem names_loop_spec (l : Slice ids.DestId) (d : ids.DestId) (i : Usize)
    (hi : i.val ≤ l.val.length)
    (hprefix : ∀ j (h : j < l.val.length), j < i.val → l.val[j] ≠ d) :
    directory.names_loop l d i ⦃ b => b ↔ d ∈ l.val ⦄ := by
  unfold directory.names_loop
  apply loop.spec_decr_nat (fun i : Usize => l.val.length - i.val)
    (fun i : Usize => i.val ≤ l.val.length ∧
      ∀ j (h : j < l.val.length), j < i.val → l.val[j] ≠ d)
  · intro i ⟨hi, hprefix⟩
    unfold directory.names_loop.body
    try dsimp only
    split_ifs with hlt
    · have hlt' : i.val < l.val.length := by scalar_tac
      step as ⟨ c, hc ⟩
      step with DestId.same_spec as ⟨ b, hb ⟩
      split_ifs with hsame
      · simp only [WP.spec_ok]
        refine ⟨fun _ => ?_, fun _ => by simp⟩
        rw [← hb.mp hsame, hc]
        exact List.getElem_mem hlt'
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
          rw [← hc]
          exact fun h => hsame (hb.mpr h)
    · simp only [WP.spec_ok]
      refine ⟨fun h => (by cases h), ?_⟩
      intro hm
      obtain ⟨j, hj, hjd⟩ := List.getElem_of_mem hm
      exact absurd hjd (hprefix j hj (by scalar_tac))
  · exact ⟨hi, hprefix⟩

/-- Membership in a destination list. -/
theorem names_spec (l : Slice ids.DestId) (d : ids.DestId) :
    directory.names l d ⦃ b => b ↔ d ∈ l.val ⦄ :=
  names_loop_spec l d 0#usize (by simp) (fun j _ hj => by simp at hj)

end Gripsack.Kernel.Directory
