import Extracted.GripsackKernel
import Protocol.CleanupDisk

/-! The last acknowledgement is from unlinking the marker, not from fsync. In
this Rust mutant it returns success there, although the marker can come back. -/
namespace Gripsack.Mutant.RecoverBeforeMarkerSync

open Gripsack.Storage Gripsack.Protocol.CleanupDisk Gripsack.Protocol.CleanupDisk.Example
open Aeneas.Std Aeneas.Std.Result gripsack_kernel

private def target := selection.SelectionIdentity.Legacy ⟨2#u64⟩
private def recordBarrier : recover.RecoverState := ⟨.SyncingRecords,
  ⟨1#u32,0#u32,0#u32,some ⟨some (.Legacy ⟨1#u64⟩),target⟩⟩,1#u32,some target,.Committed,1#u32,false,none⟩

theorem premature_finish_leaves_marker : ∃ s t disk img,
    recover.recover_step recordBarrier .Done = ok (s,.Unlink .Marker) ∧
    recover.recover_step s .Done = ok (t,.Finish .Committed) ∧
    Walk layout start [.record record,.sync,.marker] disk ∧ CrashImage disk.fs img ∧
    disk.fs.visible.link marker = none ∧ img.link marker = some 3 ∧ img.link record = none := by
  obtain ⟨disk,img,walk,power,live,old,gone⟩ := before_second_barrier_marker_can_return
  exact ⟨_,_,disk,img,rfl,rfl,walk,power,live,old,gone⟩

end Gripsack.Mutant.RecoverBeforeMarkerSync
