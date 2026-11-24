import Protocol.Leaf
import Protocol.Preservation

/-! Successful leaf replies of `host::recover::{restore,seal,execute}`. Locations are
relative to a pinned, reachable parent (S1/S4/S6); a name and an inode are separate
storage coordinates. `Value` carries the actual bytes and exact mode, or the UTF-8 link
target, never just a digest. Failed calls and lost replies are not successful replies.
The recheck and act below have no intervening writer (the explicit S5 window, not CAS).
The admitted wire/blob source and the path-to-index mapping belong to the reader. -/
namespace Gripsack.Protocol.RecoverLeaf

open Gripsack.Storage Gripsack.Protocol.Publication
open Aeneas.Std Aeneas.Std.Result gripsack_kernel

inductive Value where
  | absent
  | link (target : String)
  | file (data : FileData)
  deriving DecidableEq

def Value.ofPrior : Leaf.Prior → Value
  | .absent => .absent
  | .link t => .link t
  | .file _ blob mode => .file ⟨blob.value.bytes, mode⟩

structure Codec where
  fileId : FileData → ids.Digest
  linkId : String → ids.Digest

def Value.prior (k : Codec) : Value → recover.Prior
  | .absent => .Absent
  | .link t => .Obj (.Link (k.linkId t))
  | .file f => .Obj (.File (k.fileId f))

def Value.live (k : Codec) (v : Value) : recover.Live :=
  match v.prior k with | .Absent => .Absent | .Obj i => .Obj i

/-- The reader uses the capture codec: original permission bits, not the
private blob's 0600, participate in the restored regular-file identity. -/
def Codec.ofContext (k : Leaf.Context) : Codec := ⟨fun f => k.fileId f.bytes f.mode, k.linkId⟩

theorem Value.ofPrior_live (k : Leaf.Context) (p : Leaf.Prior) :
    (Value.ofPrior p).live (.ofContext k) = p.live k := by cases p <;> rfl

/-- S7 restricted to the two values compared in a successful recheck. -/
def Separates (k : Codec) (a b : Value) : Prop := a.live k = b.live k → a = b

def targetBytes (t : String) := t.toUTF8.data.toList

private theorem target_inj {a b : String} (h : targetBytes a = targetBytes b) : a = b := by
  have eq := Array.toList_inj.mp h
  cases a with | ofByteArray a _ =>
    cases b with | ofByteArray b _ =>
      cases a; cases b; congr

def Holds (kind : Nat → Option Kind) (img : Image) (l : Loc) : Value → Prop
  | .absent => img.link l = none
  | .link t => ∃ n, img.link l = some n ∧ kind n = some (.symlink (targetBytes t))
  | .file data => ∃ n, img.link l = some n ∧ kind n = some .regular ∧ img.file n = data

theorem Holds.unique {kind img l v w} (a : Holds kind img l v) (b : Holds kind img l w) : v = w := by
  cases v <;> cases w <;> simp only [Holds] at a b <;> grind [target_inj]

/-- One inode for *all* cuts (not a per-image choice of file/target). -/
inductive Durable (m : Machine) (l : Loc) : Value → Prop where
  | absent : StableLink m.fs l none → Durable m l .absent
  | link {n t} : StableLink m.fs l (some n) → m.kind n = some (.symlink (targetBytes t)) →
      Durable m l (.link t)
  | file {n f} : StableLink m.fs l (some n) → m.kind n = some .regular → StableFile m.fs n f →
      Durable m l (.file f)

theorem Durable.holds {m l v} (h : Durable m l v) : All m.fs (fun i => Holds m.kind i l v) := by
  intro i hi
  cases h with
  | absent hn => exact hn i hi
  | link hn hk => exact ⟨_, hn i hi, hk⟩
  | file hn hk hf => exact ⟨_, hn i hi, hk, hf i hi⟩

def KindsExtend (m t : Machine) := ∀ n k, m.kind n = some k → t.kind n = some k

theorem Durable.transport {m t l v} (h : Durable m l v) (kinds : KindsExtend m t)
    (names : ∀ a, StableLink m.fs l a → StableLink t.fs l a)
    (files : ∀ n f, m.fs.visible.link l = some n → StableFile m.fs n f → StableFile t.fs n f) :
    Durable t l v := by
  cases h with
  | absent a => exact .absent (names _ a)
  | link a b => exact .link (names _ a) (kinds _ _ b)
  | file a b c => exact .file (names _ a) (kinds _ _ b) (files _ _ a.visible c)

/-- Name separation alone would miss a write through a hard link. The dummy
binding value is immaterial: `Op.Away` tests coordinates, not their contents. -/
def Away (m : Machine) (l : Loc) (op : Leaf.Op) : Prop :=
  op.Away (.binding l 0) ∧ ∀ n, m.fs.visible.link l = some n → op.Away (.content n (m.fs.visible.file n))

private theorem frame_name {s l v} (hn : StableLink s l v) (op : Leaf.Op)
    (ha : op.Away (.binding l 0)) : StableLink (op.exec s) l v := by
  cases op with
  | bind => exact hn.bind ha
  | write => exact hn.write
  | chmod => exact hn.chmod
  | rename => exact hn.rename ha.1 ha.2
  | syncFile => exact hn.syncFile
  | syncDir => exact hn.syncDir
  | publish r a =>
    cases a with
    | Stage => exact ((hn.bind ha.1).bind ha.1).write.chmod
    | SyncFile => exact hn.syncFile
    | PublishName => exact hn.rename ha.1 ha.2
    | SyncNamespace => exact hn.syncDir
    | Complete | Refused => exact hn

theorem Durable.frame {m t l v ops} (h : Durable m l v) (disk : t.fs = Leaf.execOps m.fs ops)
    (kinds : KindsExtend m t) (away : ∀ op ∈ ops, Away m l op) : Durable t l v := by
  apply h.transport kinds
  · intro a hn
    rw [disk]
    have go {s : FS} (xs : List Leaf.Op) (hs : StableLink s l a)
        (quiet : ∀ op ∈ xs, op.Away (.binding l 0)) : StableLink (Leaf.execOps s xs) l a := by
      induction xs generalizing s with
      | nil => exact hs
      | cons op rest ih => exact ih (frame_name hs op (quiet op (by simp))) (fun o ho => quiet o (by simp [ho]))
    exact go ops hn (fun op ho => (away op ho).1)
  · intro n f named hf
    rw [disk]
    apply Leaf.exec_frame (f := .content n f) hf
    intro op ho
    simpa [hf.visible] using (away op ho).2 n named

theorem sim_all {actual normal : FS} {p : Image → Prop} (sim : Simulates id actual normal)
    (h : All normal p) : All actual p := by
  intro i hi
  obtain ⟨j, hj, eq⟩ := sim.cuts i hi
  change i = j at eq
  exact eq ▸ h j hj

theorem Durable.frame_refined {m t l v ops} (h : Durable m l v)
    (disk : Simulates id t.fs (Leaf.execOps m.fs ops)) (kinds : KindsExtend m t)
    (away : ∀ op ∈ ops, Away m l op) : Durable t l v := by
  have normal := h.frame (t := {t with fs := Leaf.execOps m.fs ops}) rfl kinds away
  exact normal.transport (t := t) (fun _ _ hk => hk)
    (fun _ hn => sim_all disk hn) (fun _ _ _ hf => sim_all disk hf)

theorem Durable.after_crash {m t l v c} (h : Durable m l v) (cut : Crash m c t) : Durable t l v := by
  cases cut with
  | kill => exact h.transport (fun _ _ hk => hk) (fun _ hn => hn) (fun _ _ _ hf => hf)
  | @power i hi =>
    apply h.transport (t := m.power i) (fun _ _ hk => hk)
    · intro _ hn j hj; exact (crash_stable.mp hj) ▸ hn _ hi
    · intro _ _ _ hf j hj; exact (crash_stable.mp hj) ▸ hf _ hi

/-- The rehash and fsync use one open descriptor; the post-fsync stat
confirms the name still points to that inode. `PinnedFile` is from the
successful open-with-metadata; bytes/mode are read there, not by reopening. -/
inductive Seal (l : Loc) : Value → Machine → Machine → Prop where
  | file {m checked synced t p slot h data} : Completed m (.openFile l slot) checked →
      PinnedFile checked p slot h → checked.fs.visible.file h.ino = data →
      Completed checked (.readFile slot data.bytes) checked → Completed checked (.syncFile slot) synced →
      Completed synced (.stat l (some (h.ino, .regular))) synced → Completed synced (.syncDir l.parent) t →
      Seal l (.file data) m t
  | link {m t target} : Completed m (.readLink l (targetBytes target)) m →
      Completed m (.syncDir l.parent) t → Seal l (.link target) m t
  | absent {m t} : Completed m (.stat l none) m → Completed m (.syncDir l.parent) t → Seal l .absent m t

theorem Seal.durable {l v m t} (s : Seal l v m t) : Durable t l v := by
  cases s with
  | file opened fd data _ file named parent =>
    cases named with | stat _ bound _ =>
      have same : _ := bound
      cases file with | syncFile actual =>
        have sealed := Preservation.checked_file_settlement fd same (Completed.syncFile actual) parent
        cases parent
        refine .file (fun i hi => (sealed i hi).1) fd.2.2 (fun i hi => ?_)
        have ⟨binding, bytes⟩ := sealed i hi
        simpa [Image.read, binding, data] using bytes
  | link read parent =>
    obtain ⟨n, hk, hn⟩ := Preservation.checked_symlink_settlement read parent
    exact .link hn hk
  | absent stat parent =>
    cases stat with | absent _ missing => exact .absent (Preservation.checked_absence_settlement missing parent)

private theorem sim_refl (s : FS) : Simulates id s s := ⟨rfl, fun i hi => ⟨i, hi, rfl⟩⟩

private theorem Seal.normalForm {l v m t} (s : Seal l v m t) :
    ∃ ops, Simulates id t.fs (Leaf.execOps m.fs ops) ∧ KindsExtend m t := by
  cases s with
  | file opened _ _ _ sync _ parent =>
    cases opened; cases sync with | @syncFile _ _ h _ =>
      cases parent
      exact ⟨[.syncFile h.ino, .syncDir l.parent], sim_refl _, fun _ _ hk => hk⟩
  | link _ parent | absent _ parent =>
    cases parent
    exact ⟨[.syncDir l.parent], sim_refl _, fun _ _ hk => hk⟩

/-- A fresh stage inode augments rather than recycles any immutable kind.
The driver owns the stage handle during the successful restore sequence. -/
def allocated (m : Machine) (n : Nat) (k : Kind) (disk : FS) : Machine :=
  {m with fs := disk, kind := fun j => if j = n then some k else m.kind j}

def installed (s : FS) (tmp l : Loc) (n : Nat) : FS := (s.rename tmp l n).syncDir l.parent

def synced (s : FS) (n mode : Nat) : FS := (s.chmod n mode).syncFile n

def StageRemovable (m : Machine) (tmp : Loc) : Prop :=
  m.fs.visible.link tmp = none ∨ ∃ n k, m.fs.visible.link tmp = some n ∧ m.kind n = some k ∧ k ≠ .directory

def stageLink (s : FS) (tmp : Loc) (n : Nat) : FS :=
  (if s.visible.link tmp = none then s else s.bind tmp none).bind tmp (some n)

/-- A successful `restore` (before returning its final read). File staging uses
`Publication.created` and arbitrary successful write-all partitions, exact fchmod,
then the *file* barrier. The subsequent check is by the destination name,
not the temporary. `installed` renames first and then syncs the same parent.
For absence a NotFound unlink is also success and still needs the barrier. -/
inductive Restore (l : Loc) (expect : Value) : Value → Machine → Machine → Prop where
  | file {m p tmp n data creationMode written} : PinnedDir m p l.parent → LiveDir m l.parent →
      tmp.parent = l.parent → tmp ≠ l → Fresh m n → StageRemovable m tmp →
      Chunks n 0 data.bytes (created ⟨tmp, l, n, data⟩ m.fs creationMode) written →
      (synced written n data.mode).visible.link tmp = some n →
      Holds m.kind (synced written n data.mode).visible l expect →
      Restore l expect (.file data) m (allocated m n .regular (installed (synced written n data.mode) tmp l n))
  | link {m p tmp n target} : PinnedDir m p l.parent → LiveDir m l.parent →
      tmp.parent = l.parent → tmp ≠ l → Fresh m n → StageRemovable m tmp →
      Holds m.kind (stageLink m.fs tmp n).visible l expect →
      Restore l expect (.link target) m (allocated m n (.symlink (targetBytes target))
        (installed (stageLink m.fs tmp n) tmp l n))
  | remove {m t u} : Holds m.kind m.fs.visible l expect → Completed m (.unlink l) u →
      Completed u (.syncDir l.parent) t → Restore l expect .absent m t
  | missing {m t} : Holds m.kind m.fs.visible l expect → Completed m (.stat l none) m →
      Completed m (.syncDir l.parent) t → Restore l expect .absent m t

theorem Restore.durable {l expect prior m t} (r : Restore l expect prior m t) : Durable t l prior := by
  cases r with
  | @file m p tmp n data creationMode written _ _ _ _ _ _ ch _ _ =>
    let spec : Recipe := ⟨tmp, l, n, data⟩
    have sim := chunked_sync_refines spec m.fs creationMode ch
    have file : StableFile (synced written n data.mode) n data := by
      intro i hi
      obtain ⟨j, hj, eq⟩ := sim.cuts i hi
      change i = j at eq
      subst j
      have sealed := crash_syncFile_file hj
      simpa [spec, stage, FS.chmod, FS.write, FS.modify, Image.chmod, Image.write] using sealed
    exact .file (fun i hi => (crash_syncDir_link hi rfl).trans (visible_rename_dst _ _ _ _))
      (by simp [allocated]) file.rename.syncDir
  | link =>
    exact .link (fun i hi => (crash_syncDir_link hi rfl).trans (visible_rename_dst _ _ _ _)) (by simp [allocated])
  | remove _ un parent =>
    cases un; cases parent
    exact .absent (fun i hi => (crash_syncDir_link hi rfl).trans (by simp [FS.bind, FS.modify, Image.bind]))
  | missing _ stat parent => exact (Seal.absent stat parent).durable

private theorem sim_install {a b : FS} {src dst : Loc} (n : Nat)
    (same : src.parent = dst.parent) (h : Simulates id a b) :
    Simulates id (installed a src dst n) (installed b src dst n) := by
  have vis : a.visible = b.visible := h.visible
  have renameVis : (a.rename src dst n).visible = (b.rename src dst n).visible := by
    simp [FS.rename, same, FS.modify, vis]
  refine ⟨renameVis, fun i hi => ?_⟩
  obtain ⟨j, hj, rfl⟩ := crash_syncDir_iff.mp hi
  have renameCut : CrashImage (b.rename src dst n) j := by
    obtain ⟨k, hk, kept⟩ := (withinDir_rename_atomic same).mp hj
    obtain ⟨u, hu, eq⟩ := h.cuts k hk
    change k = u at eq
    exact (withinDir_rename_atomic same).mpr ⟨u, hu, eq ▸ kept⟩
  exact ⟨_, crash_syncDir_iff.mpr ⟨j, renameCut, rfl⟩, by simp [renameVis]⟩

private theorem alloc_extends {m n} (fresh : Fresh m n) (k disk) : KindsExtend m (allocated m n k disk) := by
  intro j old h
  have ne : j ≠ n := fun eq => by cases eq; rw [fresh.1] at h; contradiction
  simpa [allocated, ne] using h

private theorem Restore.normalForm {l expect prior m t} (r : Restore l expect prior m t) :
    ∃ ops, Simulates id t.fs (Leaf.execOps m.fs ops) ∧ KindsExtend m t := by
  cases r with
  | @file m p tmp n data mode written _ _ same _ fresh _ ch _ _ =>
    let spec : Recipe := ⟨tmp, l, n, data⟩
    exact ⟨[.publish spec .Stage, .syncFile n, .rename tmp l n, .syncDir l.parent],
      sim_install n same (chunked_sync_refines spec m.fs mode ch), alloc_extends fresh _ _⟩
  | @link m p tmp n target _ _ _ _ fresh _ _ =>
    by_cases missing : m.fs.visible.link tmp = none
    · refine ⟨[.bind tmp (some n), .rename tmp l n, .syncDir l.parent], ?_, alloc_extends fresh _ _⟩
      simpa [allocated, stageLink, missing, installed, Leaf.execOps, Leaf.Op.exec] using
        sim_refl (installed (m.fs.bind tmp (some n)) tmp l n)
    · refine ⟨[.bind tmp none, .bind tmp (some n), .rename tmp l n, .syncDir l.parent], ?_, alloc_extends fresh _ _⟩
      simpa [allocated, stageLink, missing, installed, Leaf.execOps, Leaf.Op.exec] using
        sim_refl (installed ((m.fs.bind tmp none).bind tmp (some n)) tmp l n)
  | remove _ un parent =>
    cases un; cases parent
    exact ⟨[.bind l none, .syncDir l.parent], sim_refl _, fun _ _ hk => hk⟩
  | missing _ _ parent =>
    cases parent
    exact ⟨[.syncDir l.parent], sim_refl _, fun _ _ hk => hk⟩

/-- Checked, chronological command→reply at one admitted index. `observed`
is the concrete observation cached by `execute` for this entry; Preserve
has no `expect` field of its own. `actual` is the value rechecked before the
ack, and `landed` the actual read by name after restore's parent barrier.
Selection of the branch uses the translated Rust decision and the admitted
facts, not an arbitrary classification of a preserved file as foreign. -/
inductive Reply (k : Codec) (idx : U32) (facts : recover.EntryFacts) (l : Loc) (prior observed : Value) :
    recover.Command → recover.Event → Machine → Machine → Prop where
  | seal {m t actual} : facts.prior = prior.prior k → recover.decide facts (observed.live k) = ok .Unchanged →
      Separates k actual prior → actual.live k = observed.live k → Seal l actual m t →
      Reply k idx facts l prior observed (.Seal idx facts.dest (observed.live k) facts.prior) .Done m t
  | preserve {m t actual} : facts.prior = prior.prior k → recover.decide facts (observed.live k) = ok .Preserve →
      Separates k actual observed → actual.live k = observed.live k → Seal l actual m t →
      Reply k idx facts l prior observed (.Preserve idx facts.dest) .Done m t
  | restore {m t actual landed} : facts.prior = prior.prior k → recover.decide facts (observed.live k) = ok .Restore →
      actual.live k = observed.live k → Separates k actual observed → Restore l actual prior m t →
      Holds t.kind t.fs.visible l landed → Reply k idx facts l prior observed
        (.Mutate idx facts.dest (observed.live k) (.Restore facts.prior)) (.Landed (landed.live k)) m t

def resultValue (cmd : recover.Command) (prior observed : Value) : Value :=
  match cmd with | .Preserve .. => observed | _ => prior

private theorem unchanged {k f prior observed} (hp : f.prior = Value.prior k prior)
    (dec : recover.decide f (Value.live k observed) = ok .Unchanged) : observed.live k = prior.live k := by
  have spec := (Kernel.spec_eq_ok (Kernel.decide_spec f (observed.live k)) dec).2.2 rfl
  rw [hp] at spec
  rcases spec with ⟨ha, hb⟩ | ⟨_, ha, hb⟩ <;> rw [hb, Value.live, ha]

theorem Reply.durable {k idx facts l prior observed cmd ack m t}
    (r : Reply k idx facts l prior observed cmd ack m t) : Durable t l (resultValue cmd prior observed) := by
  cases r with
  | «seal» hp dec noCollision checked s => exact (noCollision (checked.trans (unchanged hp dec))) ▸ s.durable
  | preserve _ _ noCollision checked s => exact noCollision checked ▸ s.durable
  | restore _ _ _ _ act _ => exact act.durable

/-- Every real successful settlement has a finite `Leaf.Op` normal form:
*all* of its post-barrier crash images are simulated, even after a chunked
restore. Therefore syntactic separation suffices to frame earlier records,
selection pointers, and settled leaves through such a reply. -/
theorem Reply.normalForm {k idx facts l prior observed cmd ack m t}
    (r : Reply k idx facts l prior observed cmd ack m t) :
    ∃ ops, Simulates id t.fs (Leaf.execOps m.fs ops) ∧ KindsExtend m t := by
  cases r with
  | «seal» _ _ _ _ s | preserve _ _ _ _ s => exact s.normalForm
  | restore _ _ _ _ act _ => exact act.normalForm

theorem Reply.preserved {k idx facts l prior observed dest ack m t}
    (r : Reply k idx facts l prior observed (.Preserve idx dest) ack m t) :
    recover.decide facts (observed.live k) = ok .Preserve := by
  cases r with | preserve _ dec _ _ _ => exact dec

/-- `on_landed` checks the typed prior; honest staged restoration gives
that identity, rather than assigning it to a successful host reply by fiat. -/
theorem Reply.landed {k idx facts l prior observed cmd live m t}
    (r : Reply k idx facts l prior observed cmd (.Landed live) m t) : live = prior.live k := by
  cases r with | restore _ _ _ _ act read =>
    exact congrArg (Value.live k) (read.unique (act.durable.holds _ (crash_visible _)))

/-- `ReadCurrent` actually reads a symlink in the pinned home. Its target
is an immutable property of its inode; `Barrier Selection` seals the home
binding. `decode` stands for the admitted reader with the reserved selection
chain/manifest fixed: their durable reachability and interpretation must be
proved or premised separately. A home fsync does not sync a manifest or an
external destination file. -/
theorem committed_selection_barrier {m t : Machine} {current : Loc} {bytes}
    (decode : List UInt8 → Option selection.SelectionIdentity) {target : selection.SelectionIdentity}
    (read : Completed m (.readLink current bytes) m) (identity : decode bytes = some target)
    (barrier : Completed m (.syncDir current.parent) t) :
    ∃ n, StableLink t.fs current (some n) ∧ t.kind n = some (.symlink bytes) ∧ decode bytes = some target := by
  obtain ⟨n, hk, hn⟩ := Preservation.checked_symlink_settlement read barrier
  exact ⟨n, hn, hk, identity⟩

end Gripsack.Protocol.RecoverLeaf
