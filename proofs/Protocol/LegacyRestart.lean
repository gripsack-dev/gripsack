import Protocol.RecoverWalk
import Protocol.CleanupDisk

/-! Why a surviving undo row with no marker matters: the permitted legacy
reader can now choose the prior of an already committed value. These bytes
belong to the small `journal_cleanup.rs` test wire, not a new wire format. -/
namespace Gripsack.Protocol.LegacyRestart

open Aeneas Aeneas.Std Result gripsack_kernel Gripsack.Storage Gripsack.Protocol.RecoverWalk
open Gripsack.Protocol.CleanupDisk.Example

private def fixtureRow (idOf : FileData → ids.Digest) : recover.EntryFacts :=
  ⟨⟨0#u32⟩,.Obj (.File (idOf one)),some (.File (idOf one)),.Obj (.File (idOf two)),.Leaf⟩

def readFixture (m : Machine) (idOf : FileData → ids.Digest) :
    Option (recover.JournalFacts × recover.EntryFacts × recover.Live) := do
  if (m.fs.visible.link marker).isSome then none else
    let entry ← m.fs.visible.read record
    if entry != undo then none else
      let found ← m.fs.visible.read dest
      some (legacyOneJournal,fixtureRow idOf,.Obj (.File (idOf found)))

/-- The selected manifest and TWO are still present, but this *concrete* read
answers the actual machine with a markerless header and a stale row. No call to
`ReadCurrent` occurs on this route. The typed command requests ONE, not TWO. -/
theorem stale_row_authorizes_prior (idOf : FileData → ids.Digest) {m : Machine} (bad : Resurrected m) :
    ∃ j f live s, readFixture m idOf = some (j,f,live) ∧ one ≠ two ∧
      Prefix [(.ReadJournal,.JournalRead j),(.ReadEntry 0#u32,.EntryRead f),(.Observe 0#u32 f.dest,.Observed f.dest live)]
        s (.Mutate 0#u32 f.dest live (.Restore (.Obj (.File (idOf one))))) := by
  obtain ⟨gone,entry,found,_⟩ := bad
  let f := fixtureRow idOf
  let l := recover.Live.Obj (.File (idOf two))
  let initial : recover.RecoverState := ⟨.Start,⟨0#u32,0#u32,0#u32,none⟩,0#u32,none,.Pending,0#u32,false,none⟩
  let reading := {initial with phase := .ReadingJournal}
  let row : recover.RecoverState := ⟨.ReadingEntry,legacyOneJournal,1#u32,none,.Uncommitted,0#u32,false,none⟩
  let observed := {row with phase := .Observing, entry := some f}
  refine ⟨legacyOneJournal,f,l,{observed with phase := .Restoring},?_,by decide,?_⟩
  · simp [readFixture,gone,entry,found,f,l]
  have add : U32.checked_add 1#u32 0#u32 = some 1#u32 := by decide
  have p : Prefix [] reading .ReadJournal := .start (initial := initial) rfl rfl
  have h := p.answer (e := .JournalRead legacyOneJournal) (t := row) (d := .ReadEntry 0#u32)
    (by simp [IsCleanup]) trivial (by simp [reading,initial,row,legacyOneJournal,recover.recover_step,
      recover.on_journal,recover.proceed,lift,add])
  have h' := h.answer (e := .EntryRead f) (t := observed) (d := .Observe 0#u32 f.dest)
    (by simp [IsCleanup]) trivial rfl
  apply h'.answer (e := .Observed f.dest l) (by simp [IsCleanup]) rfl
  obtain ⟨b,call,iff⟩ := Kernel.spec_exists_ok (Kernel.intends_spec f.intended (.File (idOf two)))
  have yes : b = true := iff.mpr rfl
  subst b
  simp [observed,row,f,fixtureRow,l,recover.recover_step,recover.on_observed,ids.DestId.same,
    recover.decide,show recover.intends (.Obj (.File (idOf two))) (.File (idOf two)) = ok true from call]

end Gripsack.Protocol.LegacyRestart
