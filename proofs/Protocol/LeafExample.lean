import Protocol.Leaf

/-! A complete one-unit run, ending at issuance (not execution) of `Mutate`.
The initial visible destination is a regular file. The journal and prior
namespace start absent; capture creates `prior/` and takes the blob-miss
path. The tiny partial decoders below recognize only this fixture's bytes;
they exercise the host contract, not the Rust wire parser. -/
namespace Gripsack.Protocol.Leaf.Example

open Gripsack.Storage Gripsack.Protocol.Publication Gripsack.Protocol.Leaf
open Gripsack.Kernel Gripsack.Kernel.JournalRun
open gripsack_kernel Aeneas Aeneas.Std Result

private def journalDir : Directory := ⟨⟨0, 1⟩, 1⟩
private def priorDir : Directory := ⟨⟨0, 2⟩, 2⟩
private def leafLoc : Loc := ⟨0, 3⟩
private def marker : Recipe := ⟨⟨1, 1⟩, ⟨1, 2⟩, 4, ⟨[77], 0o600⟩⟩
private def blob : Recipe := ⟨⟨2, 1⟩, ⟨2, 2⟩, 5, ⟨[42], 0o600⟩⟩
private def entry : Recipe := ⟨⟨1, 3⟩, ⟨1, 4⟩, 6, ⟨[69], 0o600⟩⟩
private def origMode := 0o644
private def home : Image := ⟨fun k => if k = leafLoc then some 3 else none,
  fun n => if n = 3 then ⟨blob.value.bytes, origMode⟩ else ⟨[], 0⟩⟩
private def base := FS.stable home
private def disk := (base.bind journalDir.loc (some journalDir.inode)).syncDir journalDir.loc.parent
private def rawHash : ids.Digest := ⟨Aeneas.Std.Array.repeat 32#usize 1#u8⟩
private def objHash : ids.Digest := ⟨Aeneas.Std.Array.repeat 32#usize 2#u8⟩
private def target : selection.SelectionIdentity := .Transaction ⟨1#u64⟩ ⟨Aeneas.Std.Array.repeat 32#usize 3#u8⟩
private def dest : ids.DestId := ⟨0#u32⟩
private def live : recover.Live := .Obj (.File objHash)
private def facts : journal_run.UnitFacts := ⟨dest, live, .Removed, none⟩
private def path := "rc"
private def decoded : Entry := ⟨path, .file rawHash origMode, .file rawHash origMode, .Removed⟩
private def ctx : Context where
  journal := journalDir
  priorDir := priorDir.loc
  marker := marker
  inside := rfl
  previous := none
  target := target
  path := fun _ => path
  facts := fun _ => facts
  hash := fun _ => rawHash
  fileId := fun _ _ => objHash
  linkId := fun _ => objHash
  blobName := fun _ => blob.dst.name
  entryName := fun _ => entry.dst.name
  decodeMarker := fun bs => if bs = [77] then some (none, target) else none
  decodeEntry := fun bs => if bs = [69] then some decoded else none
  markerWire := rfl

private def prior := Prior.file priorDir blob origMode
private def packet : Packet := ⟨0#u32, path, prior, prior, entry⟩
private theorem packet_matches : Matches ctx packet := by
  refine ⟨rfl, rfl, rfl, ?_⟩
  intro d r m eq
  cases eq
  exact ⟨rfl, rfl, rfl, by decide, by decide⟩

/-! The allocator never picks an inode reachable in *any* allowed crash
image. Carry that negative fact through the actual earlier operations. -/
private def Unused (s : FS) (n : Nat) := All s (fun i => ∀ l, i.link l ≠ some n)

private theorem unusedBind {s n l v} (h : Unused s n) (hv : v ≠ some n) : Unused (s.bind l v) n := by
  apply All.modify h
  intro i hi k
  by_cases e : k = l <;> simp [Image.bind, e, hv, hi]

private theorem unusedDir {s n} (h : Unused s n) (d) : Unused (s.syncDir d) n := by
  intro i hi k
  obtain ⟨j, hj, rfl⟩ := crash_syncDir_iff.mp hi
  by_cases e : k.parent = d <;> simp [Image.copyDir, e, h j hj k, h _ (crash_visible s) k]

private theorem unusedPublished {s n} (h : Unused s n) (r : Recipe)
    (same : r.tmp.parent = r.dst.parent) (ne : r.inode ≠ n) : Unused (published r s) n := by
  have hv : some r.inode ≠ some n := by simpa using ne
  have h₁ := unusedBind (unusedBind (l := r.tmp) h (v := none) (by simp)) (l := r.tmp) hv
  have h₂ : Unused (stage r s) n :=
    All.modify (All.modify h₁ (f := fun i => i.write r.inode r.value.bytes) (fun _ hi => hi))
      (fun _ hi => hi)
  have h₃ : Unused (sealed r s) n := by
    intro i hi k
    obtain ⟨j, hj, he⟩ := crash_syncFile_links hi
    rw [he]; exact h₂ j hj k
  apply unusedDir (s := renamed r s)
  intro i hi k
  obtain ⟨j, hj, rfl | rfl⟩ := (withinDir_rename_atomic same).mp hi
  · exact h₃ _ hj k
  · have old := h₃ j hj k
    unfold Image.rename
    split <;> simp only [Image.bind] <;> (repeat' split) <;> first | exact hv | exact old | simp

private theorem unusedDisk {n : Nat} (h₁ : journalDir.inode ≠ n) (h₃ : 3 ≠ n) : Unused disk n := by
  apply unusedDir
  apply unusedBind _ (by simpa using h₁)
  intro i hi k
  obtain rfl := crash_stable.mp hi
  by_cases e : k = leafLoc <;> simp [home, e, h₃]

private def afterMarker := published marker disk
private def pre : List Op := [.bind priorDir.loc (some priorDir.inode)]
private def blobBase := (execOps afterMarker pre).syncDir priorDir.loc.parent
private def ops : List Op := pre ++ [.syncDir priorDir.loc.parent, .publish blob .Stage,
  .publish blob .SyncFile, .publish blob .PublishName, .publish blob .SyncNamespace]
private def afterBlob := published blob blobBase
private theorem unusedBlobBase {n : Nat} (h₁ : journalDir.inode ≠ n) (h₂ : priorDir.inode ≠ n)
    (h₃ : 3 ≠ n) (h₄ : marker.inode ≠ n) : Unused blobBase n := by
  apply unusedDir
  exact unusedBind (unusedPublished (unusedDisk h₁ h₃) marker rfl h₄) (by simpa using h₂)

private theorem captured : Capture prior afterMarker ops :=
  .miss afterMarker priorDir blob origMode pre (by rfl) rfl
    ⟨rfl, by decide, unusedBlobBase (by decide) (by decide) (by decide) (by decide)⟩

private def trace := pubSteps .Marker .Start .SyncNamespace ++ [(.Done, .Ready)] ++
  leafLog facts 0#u32 live .SyncNamespace
private def cut : World := ⟨published entry afterBlob, false, [packet]⟩

/-! Forward interpretation of the four publication requests. The protected
footprint is determined by the call and history, not by the disk itself. -/
private theorem pubPrefix {k w rec r e tail t last}
    (sel : selected k w rec = some r) (fresh : Enabled r .Stage w.disk)
    (apart : ∀ f, Protected k w f → (Op.publish r .Stage).Away f)
    (rest : Walk k {w with disk := published r w.disk} .Done tail t last) :
    Walk k w e (pubSteps rec e .SyncNamespace ++ tail) t last := by
  have step {w' : World} {ev a} (hs : selected k w' rec = some r) (en : Enabled r a w'.disk)
      (ha : ∀ f, Protected k w' f → (Op.publish r a).Away f) :
      Answer k w' ev (.Publish rec a) .Done {w' with disk := effect r a w'.disk} := by
    have act : Host k w' ev (.Publish rec a) .Done ⟨[.publish r a], none, false⟩ := .publish hs en
    have ans := Answer.done act (by
      intro f hf
      exact Or.inr (by intro op hm; obtain rfl := List.mem_singleton.mp hm; exact ha f hf))
    simpa [Receipt.apply, execOps, Op.exec] using ans
  have tmp : (stage r w.disk).visible.link r.tmp = some r.inode := by
    simp [stage, FS.bind, FS.write, FS.chmod, FS.modify, Image.bind, Image.write, Image.chmod]
  have selectedAt (s : FS) : selected k {w with disk := s} rec = some r := by cases rec <;> exact sel
  have awayAt (s : FS) (a : Action) (f) (hf : Protected k {w with disk := s} f) :
      (Op.publish r a).Away f := by cases f <;> exact apart _ hf
  exact .cons (step sel fresh apart)
    (.cons (step (a := .SyncFile) (selectedAt (stage r w.disk)) tmp (awayAt _ _))
    (.cons (step (a := .PublishName) (selectedAt (sealed r w.disk)) tmp (awayAt _ _))
    (.cons (step (a := .SyncNamespace) (selectedAt (renamed r w.disk)) (visible_rename_dst _ _ _ _) (awayAt _ _)) rest)))

local macro "apart_decide" : tactic => `(tactic| (simp only [ctx, Directory.fact, Op.Away] <;> decide))

private theorem markerApart (f) (hf : Protected ctx (Leaf.initial disk) f) :
    (Op.publish marker .Stage).Away f := by
  rcases hf with rfl | ⟨hb, _⟩ | ⟨_, hm, _⟩
  · apart_decide
  · cases hb
  · cases hm

private theorem captureApart (f) (hf : Protected ctx ⟨afterMarker, false, []⟩ f) :
    ∀ op ∈ ops, op.Away f := by
  rcases hf with rfl | ⟨_, hm⟩ | ⟨_, hm, _⟩
  · simp only [ops, pre, List.cons_append, List.nil_append, List.mem_cons, List.not_mem_nil,
      or_false, forall_eq_or_imp, forall_eq]; apart_decide
  · change f ∈ recordFacts marker at hm
    simp only [recordFacts, List.mem_cons, List.not_mem_nil, or_false] at hm
    rcases hm with rfl | rfl <;>
      simp only [ops, pre, List.cons_append, List.nil_append, List.mem_cons, List.not_mem_nil,
        or_false, forall_eq_or_imp, forall_eq] <;> apart_decide
  · cases hm

private theorem entryApart (f) (hf : Protected ctx ⟨afterBlob, false, [packet]⟩ f) :
    (Op.publish entry .Stage).Away f := by
  rcases hf with rfl | ⟨_, hm⟩ | ⟨q, hq, hm⟩
  · apart_decide
  · change f ∈ recordFacts marker at hm
    simp only [recordFacts, List.mem_cons, List.not_mem_nil, or_false] at hm
    rcases hm with rfl | rfl <;> apart_decide
  · obtain rfl := List.mem_singleton.mp hq
    change f ∈ prior.facts ++ prior.facts at hm
    simp only [List.mem_append, or_self, prior, Prior.facts, recordFacts, List.mem_cons,
      List.not_mem_nil, or_false] at hm
    rcases hm with rfl | rfl | rfl <;> apart_decide

private theorem hostWalk : Walk ctx (Leaf.initial disk) .Start trace cut .Done := by
  unfold trace
  rw [List.append_assoc]
  apply pubPrefix (k := ctx) (w := Leaf.initial disk) (rec := .Marker) (r := marker) rfl
    ⟨rfl, by decide, unusedDisk (n := marker.inode) (by decide) (by decide)⟩ markerApart
  refine .cons (.done (.ready _ facts) ?_) ?_
  · intro _ _; exact Or.inr (by simp)
  refine .cons (.done (.observe 0#u32 live) ?_) ?_
  · intro _ _; exact Or.inr (by simp)
  refine .cons (.done (.capture _ packet ops packet_matches .first captured) ?_) ?_
  · intro f hf; exact Or.inr (captureApart f hf)
  apply pubPrefix (k := ctx) (w := ⟨afterBlob, false, [packet]⟩) (rec := .Entry) (r := entry)
    (tail := []) rfl ⟨rfl, by decide, ?_⟩ entryApart (.nil _ _)
  exact unusedPublished (unusedBlobBase (n := entry.inode) (by decide) (by decide) (by decide) (by decide)) blob rfl (by decide)

private def st (phase : journal_run.Phase) (step : Action) (unit : Option journal_run.UnitFacts := none)
    (observed : recover.Live := .Absent) : State := ⟨phase, step, false, 0#u32, unit, observed, 0#u32, 0#u32⟩
private def beforeMutation := st .PublishingEntry .SyncNamespace (some facts) live
private def mutating := st .Mutating .SyncNamespace (some facts) live

local macro "next_request" : tactic => `(tactic|
  (simp only [journal_run.run_step, journal_run.on_marker_step, journal_run.on_entry_step,
    beforeMutation, mutating, st, journal.PublicationAction.successor, Std.bind_ok] <;> rfl))

private theorem kernelRun : Run beforeMutation trace := by
  have r₀ : Run (st .Start .Stage) [] := .start rfl
  have r₁ := Run.step (s' := st .PublishingMarker .Stage) (e := .Start) r₀ (by rfl)
  have r₂ := Run.step (s' := st .PublishingMarker .SyncFile) (e := .Done) r₁ (by next_request)
  have r₃ := Run.step (s' := st .PublishingMarker .PublishName) (e := .Done) r₂ (by next_request)
  have r₄ := Run.step (s' := st .PublishingMarker .SyncNamespace) (e := .Done) r₃ (by next_request)
  have r₅ := Run.step (s' := st .Ready .SyncNamespace) (e := .Done) r₄ (by next_request)
  have r₆ := Run.step (s' := st .Observing .SyncNamespace (some facts)) (e := .Unit facts) r₅ (by rfl)
  obtain ⟨b, hb, correct⟩ := spec_exists_ok (Live.same_spec live live)
  have same : recover.Live.same live live = ok true := hb.trans (congrArg ok (correct.mpr rfl))
  obtain ⟨d, hd, correctDest⟩ := spec_exists_ok (DestId.same_spec dest dest)
  have sameDest : ids.DestId.same dest dest = ok true := hd.trans (congrArg ok (correctDest.mpr rfl))
  have observed : journal_run.run_step (st .Observing .SyncNamespace (some facts)) (.Observed dest live) =
      ok (st .Capturing .SyncNamespace (some facts) live, .CapturePrior 0#u32 dest live) := by
    simp only [journal_run.run_step, st, journal_run.on_observed, facts, sameDest, Std.bind_ok,
      ite_true, live, show recover.Live.same (.Obj (.File objHash)) (.Obj (.File objHash)) = ok true from same,
      journal_run.follows]
  have r₇ := Run.step r₆ observed
  have r₈ := Run.step (s' := st .PublishingEntry .Stage (some facts) live) (e := .Done) r₇ (by rfl)
  have r₉ := Run.step (s' := st .PublishingEntry .SyncFile (some facts) live) (e := .Done) r₈ (by next_request)
  have r₁₀ := Run.step (s' := st .PublishingEntry .PublishName (some facts) live) (e := .Done) r₉ (by next_request)
  exact Run.step (s' := beforeMutation) (e := .Done) r₁₀ (by next_request)

/-- Positive admission: from a home with just `rc`, the translated run
actually issues removal of that file. Killing the process or taking any
admitted power image at this cut leaves both directory names, a decodable
marker and entry under their own names, and the named blob's original bytes. -/
theorem reachable_file_removal :
    base.visible.read leafLoc = some ⟨[42], origMode⟩ ∧
    base.visible.link journalDir.loc = none ∧ base.visible.link priorDir.loc = none ∧
    Prepare ctx.journal base disk ∧ Run beforeMutation trace ∧
    Walk ctx (Leaf.initial disk) .Start trace cut .Done ∧
    journal_run.run_step beforeMutation .Done = ok (mutating, .Mutate 0#u32 dest live .Removed) ∧
    ∀ i, CrashImage cut.disk i → i.link journalDir.loc = some journalDir.inode ∧
      (i.read marker.dst).bind (fun v => ctx.decodeMarker v.bytes) = some (none, target) ∧
      (i.read entry.dst).bind (fun v => ctx.decodeEntry v.bytes) = some decoded ∧
      i.link priorDir.loc = some priorDir.inode ∧ i.read blob.dst = some ⟨[42], 0o600⟩ := by
  have parent : Prepare ctx.journal base disk := .created rfl
  have mutation : journal_run.run_step beforeMutation .Done = ok (mutating, .Mutate 0#u32 dest live .Removed) := by next_request
  refine ⟨rfl, rfl, rfl, parent, kernelRun, hostWalk, mutation, ?_⟩
  obtain ⟨q, hq, _, _, _, _, survives⟩ := mutation_evidence_survives parent kernelRun hostWalk mutation
  obtain rfl := List.mem_singleton.mp hq
  intro i hi
  obtain ⟨hj, hm, he, _, _, hb⟩ := survives i hi
  exact ⟨hj, hm, he, hb⟩

end Gripsack.Protocol.Leaf.Example
