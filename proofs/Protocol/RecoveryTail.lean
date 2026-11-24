import Protocol.CleanupOrder
import Protocol.CleanupDisk

/-! No independent cleanup scheduler. The disk operation is selected from
`recover_step`'s actual outstanding command, the host's ordinal/name table,
and the pinned journal. Only a successful physical reply can be continued. -/
namespace Gripsack.Protocol.RecoveryTail

open Gripsack.Storage Gripsack.Protocol.CleanupOrder Gripsack.Protocol.CleanupDisk
open Aeneas.Std Aeneas.Std.Result gripsack_kernel

abbrev Physical (j : Layout) := operation (j.records.map Op.record) Op.sync Op.marker
abbrev Remaining (j : Layout) := Cursor (j.records.map Op.record) Op.sync Op.marker

inductive Execution (j : Layout) : State → Cmd → Machine → Machine → Prop where
  | pause {s c m} : Execution j s c m m
  | interrupted {s c m op u status} : Physical j c = some op → Call j m op status u → status ≠ .ok →
      Execution j s c m u
  | reply {s c m op u t d last} : Physical j c = some op → Call j m op .ok u →
      recover.recover_step s .Done = ok (t,d) → Execution j t d u last → Execution j s c m last

/-- Like `Execution` but ends only at an actual `Finish`, not an arbitrary cut. -/
inductive Complete (j : Layout) : State → Cmd → Machine → Machine → Prop where
  | finished {s m o} : Complete j s (.Finish o) m m
  | reply {s c m op u t d last} : Physical j c = some op → Call j m op .ok u →
      recover.recover_step s .Done = ok (t,d) → Complete j t d u last → Complete j s c m last

private theorem uncons {j s c ops op} (h : Remaining j s c ops) (issued : Physical j c = some op) :
    ∃ tail, ops = op :: tail := by
  have head := h.head.symm.trans issued
  cases ops with
  | nil => cases head
  | cons a rest => exact ⟨rest, by cases Option.some.inj head; rfl⟩

theorem Execution.cut {j s c m t} (run : Execution j s c m t) {ops} (h : Remaining j s c ops) :
    Cut j m ops t := by
  induction run generalizing ops with
  | pause => exact .stop
  | interrupted issued call bad =>
    obtain ⟨_,rfl⟩ := uncons h issued
    exact .interrupted call bad
  | reply issued call step more ih =>
    obtain ⟨_,rfl⟩ := uncons h issued
    exact .next call (ih (h.advance step))

theorem Complete.walk {j s c m t} (run : Complete j s c m t) {ops} (h : Remaining j s c ops) :
    Walk j m ops t := by
  induction run generalizing ops with
  | finished => cases h; exact .nil
  | reply issued call step more ih =>
    obtain ⟨_,rfl⟩ := uncons h issued
    exact .cons call (ih (h.advance step))

end Gripsack.Protocol.RecoveryTail
