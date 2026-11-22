import Kernel.Recover

/-! Read the physical unlink/barrier list from the outstanding translated Rust command.
The typed list may contain any number of record names; the two journal barriers
are distinguished by their issuing phases, not by different syscall semantics. -/
namespace Gripsack.Protocol.CleanupOrder

open Aeneas Aeneas.Std Result gripsack_kernel Gripsack.Kernel

abbrev State := recover.RecoverState
abbrev Cmd := recover.Command

/-- The host's frozen `record_name(index)` table, with the name effects already
mapped to `α`. No clock or synthetic state transition is used here. -/
def operation (records : List α) (sync marker : α) : Cmd → Option α
  | .Unlink (.Record i) => records[i.val]?
  | .Unlink .Marker => some marker
  | .Barrier .Journal => some sync
  | _ => none

inductive Cursor (records : List α) (sync marker : α) : State → Cmd → List α → Prop where
  | record {s : State} (phase : s.phase = .UnlinkingRecord)
      (count : s.records.val = records.length) (within : s.index.val < s.records.val) :
      Cursor records sync marker s (.Unlink (.Record s.index))
        (records.drop s.index.val ++ [sync, marker, sync])
  | first {s : State} (phase : s.phase = .SyncingRecords) :
      Cursor records sync marker s (.Barrier .Journal) [sync, marker, sync]
  | markerName {s : State} (phase : s.phase = .UnlinkingMarker) :
      Cursor records sync marker s (.Unlink .Marker) [marker, sync]
  | last {s : State} (phase : s.phase = .SyncingMarker) :
      Cursor records sync marker s (.Barrier .Journal) [sync]
  | done {s : State} {o} (phase : s.phase = .Finished) :
      Cursor records sync marker s (.Finish o) []

/-- The interpreted current operation is exactly the head of the remaining
list. In particular a read or a failed command cannot stand in for a barrier. -/
theorem Cursor.head {r : List α} {b m s c ops} (h : Cursor r b m s c ops) :
    operation r b m c = ops.head? := by
  cases h with
  | record ph count bound =>
    have hin : _ < r.length := bound.trans_eq count
    rw [List.drop_eq_getElem_cons hin]
    simp [operation, hin]
  | first | markerName | last | done => rfl

private theorem next_cursor {r : List α} {b m} (s : State) {t c}
    (count : s.records.val = r.length) (h : recover.next_unlink s = ok (t,c)) :
    Cursor r b m t c (r.drop s.index.val ++ [b,m,b]) := by
  rcases spec_eq_ok (next_unlink_spec s) h with ⟨lt, eq⟩ | ⟨le, eq⟩
  · cases eq
    exact .record rfl count lt
  · cases eq
    rw [List.drop_eq_nil_of_le (count ▸ le), List.nil_append]
    exact .first rfl

/-- A successful storage acknowledgement advances exactly one position of this
list. Proved over `recover_step`, not an independently executable cleanup loop. -/
theorem Cursor.advance {r : List α} {b m s c a tail t d}
    (h : Cursor r b m s c (a :: tail)) (step : recover.recover_step s .Done = ok (t,d)) :
    Cursor r b m t d tail := by
  generalize eq : a :: tail = ops at h
  cases h with
  | record ph count bound =>
    have hin : s.index.val < r.length := bound.trans_eq count
    rw [List.drop_eq_getElem_cons hin] at eq
    have rest := (List.cons.inj eq).2
    rw [rest]
    simp only [recover.recover_step, ph] at step
    have plus := U32.checked_add_bv_spec s.index 1#u32
    cases add : U32.checked_add s.index 1#u32 with
    | none =>
      rw [add] at plus
      simp at plus
      scalar_tac
    | some i =>
      rw [add] at plus
      have inc : i.val = s.index.val + 1 := by simpa using plus.2.1
      simp only [add, lift, Std.bind_ok] at step
      rw [← inc]
      exact next_cursor {s with index := i} count step
  | first ph =>
    cases eq
    simp only [recover.recover_step, ph, ok.injEq, Prod.mk.injEq] at step
    obtain ⟨rfl,rfl⟩ := step
    exact .markerName rfl
  | markerName ph =>
    cases eq
    simp only [recover.recover_step, ph, ok.injEq, Prod.mk.injEq] at step
    obtain ⟨rfl,rfl⟩ := step
    exact .last rfl
  | last ph =>
    cases eq
    simp only [recover.recover_step, ph] at step
    obtain ⟨o,eq'⟩ := spec_eq_ok (finish_spec _) step
    cases eq'
    exact .done rfl
  | done _ => cases eq

/-- Errors, even errors reported after a physical effect, do not authorize the
next cleanup command. A killed process has no acknowledgement at all. -/
theorem Cursor.failed {r : List α} {b m s c a tail t d failure}
    (h : Cursor r b m s c (a :: tail))
    (step : recover.recover_step s (.Failed failure) = ok (t,d)) : d = .Abort .Failed := by
  generalize eq : a :: tail = ops at h
  cases h with
  | record ph _ _ | first ph | markerName ph | last ph =>
    simp [recover.recover_step, ph, recover.failed_or_protocol, recover.refuse] at step
    exact step.2.symm
  | done _ => cases eq

end Gripsack.Protocol.CleanupOrder
