import Protocol.Publication

/-! Calibrated only against the retranslated `run-mutates-before-entry-barrier` Rust mutant.
The ordinary kernel order theorem must fail first. Independently, replay the translated
machine from `new` on a fresh journal: at its first issued mutation the marker is present,
the entry inode has the intended bytes, but the entry name can be absent. -/
namespace Gripsack.Mutant.RunBeforeEntryBarrier

open Gripsack.Storage Gripsack.Protocol.Publication gripsack_kernel Aeneas Aeneas.Std Result

private def marker : Recipe := ⟨⟨1, 4⟩, ⟨1, 5⟩, 10, ⟨[77], 384⟩⟩
private def dest : ids.DestId := ⟨0#u32⟩
private def journalName : Loc := ⟨0, 1⟩
private def home := FS.stable (freshImage.bind journalName (some 1))

-- Our `do` uses Lean's `Bind.bind`; the translation uses `Std.bind` directly.
private theorem replay_bind_ok (x : α) (f : α → Result β) : (ok x >>= f) = f x :=
  Std.bind_ok x f

/-- Interpret only the issued publications. Observing and capturing absence need no
blob write. Stop at admission, before performing the destination mutation. -/
private def replay (s : journal_run.RunState) (disk : FS) : List journal_run.Event →
    Result (List journal_run.Command × FS)
  | [] => ok ([], disk)
  | e :: es => do
    let issued ← journal_run.run_step s e
    let d := match issued.2 with
      | .Publish .Marker a => effect marker a disk
      | .Publish .Entry a => effect exampleRecord a disk
      | _ => disk
    let rest ← replay issued.1 d es
    ok (issued.2 :: rest.1, rest.2)

private def run (intent : recover.Intended) := do
  let start ← journal_run.RunState.new
  replay start home [.Start, .Done, .Done, .Done, .Done,
    .Unit ⟨dest, .Absent, intent, none⟩, .Observed dest .Absent, .Done, .Done, .Done, .Done]

private def commands (intent : recover.Intended) : List journal_run.Command :=
  [.Publish .Marker .Stage, .Publish .Marker .SyncFile, .Publish .Marker .PublishName,
   .Publish .Marker .SyncNamespace, .Ready, .Observe 0#u32 dest, .CapturePrior 0#u32 dest .Absent,
   .Publish .Entry .Stage, .Publish .Entry .SyncFile, .Publish .Entry .PublishName,
   .Mutate 0#u32 dest .Absent intent]

theorem mutation_with_lost_entry (intent : recover.Intended) :
    ∃ disk, run intent = ok (commands intent, disk) ∧ ∃ i, CrashImage disk i ∧
      i.link journalName = some 1 ∧ i.read marker.dst = some marker.value ∧ i.link exampleRecord.dst = none ∧
      i.file exampleRecord.inode = exampleRecord.value := by
  let r := exampleRecord
  let base := published marker home
  refine ⟨renamed r base, ?_, ?_⟩
  · have same : ids.DestId.same dest dest = ok true := by simp [ids.DestId.same]
    simp only [run, journal_run.RunState.new, replay, journal_run.run_step, journal_run.on_marker_step,
      journal_run.on_unit, journal_run.on_observed, same, recover.Live.same, journal_run.follows,
      journal_run.on_captured, journal_run.on_entry_step, journal.PublicationAction.successor,
      replay_bind_ok, Std.bind_ok, ite_true, commands]
    rfl
  have staged : CrashImage (stage r base) base.visible :=
    crash_modify_skip (crash_modify_skip (crash_modify_skip (crash_modify_skip (crash_visible base))))
  let power := base.visible.copyFile (stage r base).visible r.inode
  have sealedCut : CrashImage (sealed r base) power := crash_syncFile_iff.mpr ⟨_, staged, rfl⟩
  exact ⟨power, (withinDir_rename_atomic (s := sealed r base) (src := r.tmp) (dst := r.dst)
    (n := r.inode) rfl).mpr ⟨_, sealedCut, Or.inl rfl⟩, by decide, by decide, by decide, by decide⟩

end Gripsack.Mutant.RunBeforeEntryBarrier
