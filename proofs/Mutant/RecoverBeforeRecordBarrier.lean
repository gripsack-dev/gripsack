import Extracted.GripsackKernel
import Protocol.CleanupDisk

/-! Positive counterexample checked only against the freshly retranslated
`recover-cleanup-barrier-swapped` source. The journal operations come from
the actual issued commands, not a handwritten two-unlink input to the test. -/
namespace Gripsack.Mutant.RecoverBeforeRecordBarrier

open Gripsack.Storage Gripsack.Protocol.CleanupDisk Gripsack.Protocol.CleanupDisk.Example
open Aeneas.Std Aeneas.Std.Result gripsack_kernel

private def target := selection.SelectionIdentity.Legacy ⟨2#u64⟩
private def header : recover.JournalFacts := ⟨1#u32,0#u32,0#u32,some ⟨some (.Legacy ⟨1#u64⟩),target⟩⟩
private def journalAnswer : recover.Event :=
  if start.fs.visible.read marker = some run ∧ start.fs.visible.read record = some undo then .JournalRead header else .Failed .Drift
private def currentAnswer : recover.Event :=
  if start.fs.visible.link current = some 5 ∧ start.kind 5 = some (.symlink (bytes "generations/2")) ∧
    start.fs.visible.read manifest = some selected then .CurrentRead (some target) else .Failed .Drift

private def replay (s : recover.RecoverState) : List recover.Event → Result (List recover.Command)
  | [] => ok []
  | e :: es => do
    let (next,cmd) ← recover.recover_step s e
    let rest ← replay next es
    ok (cmd :: rest)

private def issued := do
  let s ← recover.RecoverState.new
  replay s [.Start,journalAnswer,currentAnswer,.Done,.Done]

private def journalOp : recover.Command → Option Op
  | .Unlink (.Record i) => layout.records[i.val]?.map Op.record
  | .Unlink .Marker => some .marker
  | .Barrier .Journal => some .sync
  | _ => none

theorem issued_marker_allows_stale_row : ∃ cmds m reboot, issued = ok cmds ∧
    cmds = [.ReadJournal,.ReadCurrent,.Barrier .Selection,.Unlink (.Record 0#u32),.Unlink .Marker] ∧
    Completed start (.syncDir current.parent) start ∧ Walk layout start (cmds.filterMap journalOp) m ∧
    Crash m .power reboot ∧ Resurrected reboot := by
  obtain ⟨m,reboot,walk,_,_,power,bad⟩ := historical_0192_before_trailing_sync false
  refine ⟨_,m,reboot,?_,rfl,?_,?_,power,bad⟩
  · have j : journalAnswer = .JournalRead header := ite_eq_left ⟨rfl,rfl⟩
    have c : currentAnswer = .CurrentRead (some target) := ite_eq_left ⟨rfl,rfl,rfl⟩
    have a : U32.checked_add 1#u32 0#u32 = some 1#u32 := by decide
    have b : U32.checked_add 0#u32 1#u32 = some 1#u32 := by decide
    simp [issued,replay,recover.RecoverState.new,j,c,header,target,recover.recover_step,
      recover.on_journal,recover.on_current,recover.begin_cleanup,recover.next_unlink,
      selection.classify,selection.SelectionIdentity.same,generation.GenerationId.same,
      lift,a,b,Aeneas.Std.bind_ok]
  · have eq (img : Image) (d : Nat) : (FS.stable img).syncDir d = FS.stable img := by
      simp [FS.syncDir,FS.barrier,FS.stable,CrashImage]
    have complete := Completed.syncDir (m := start) (d := current.parent)
      (p := (Session.root 1).pin 0) ⟨rfl,Or.inr rfl,rfl⟩
    simpa [start,eq] using complete
  · exact walk

end Gripsack.Mutant.RecoverBeforeRecordBarrier
