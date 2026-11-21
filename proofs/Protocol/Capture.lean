import Protocol.Publication

/-! Successful host effects for `Journal::prepare` and `store_blob`. A receipt
below is an equality at one storage coordinate, not an acknowledgement. Its
truth in all crash images is proved from the corresponding file/parent sync.
Names are interned relative to a pinned directory; the home itself is the
stable operator-supplied root. -/
namespace Gripsack.Protocol.Leaf

open Gripsack.Storage Gripsack.Protocol.Publication
open gripsack_kernel

inductive Fact where
  | binding (loc : Loc) (inode : Nat)
  | content (inode : Nat) (data : FileData)
  deriving DecidableEq

def Fact.Holds : Fact → Image → Prop
  | .binding loc n, i => i.link loc = some n
  | .content n data, i => i.file n = data

def Fact.Stable (f : Fact) (s : FS) := All s f.Holds

structure Directory where
  loc : Loc
  inode : Nat

def Directory.fact (d : Directory) := Fact.binding d.loc d.inode

def recordFacts (r : Recipe) : List Fact := [.binding r.dst r.inode, .content r.inode r.value]

/-- The host operations relevant to the private evidence footprint. A leaf
install, stale-temp unlink, directory create or corrupt-blob quarantine can
have many such operations. A directory barrier is not a file barrier. -/
inductive Op where
  | bind (l : Loc) (v : Option Nat)
  | write (n : Nat) (b : List UInt8)
  | chmod (n mode : Nat)
  | rename (src dst : Loc) (n : Nat)
  | syncFile (n : Nat)
  | syncDir (d : Nat)
  | publish (r : Recipe) (a : journal.PublicationAction)

def Op.exec (s : FS) : Op → FS
  | .bind l v => s.bind l v
  | .write n b => s.write n b
  | .chmod n m => s.chmod n m
  | .rename src dst n => s.rename src dst n
  | .syncFile n => s.syncFile n
  | .syncDir d => s.syncDir d
  | .publish r a => effect r a s

def execOps (s : FS) (ops : List Op) := ops.foldl Op.exec s

/-- A syntactic footprint on names/inodes, not an assumption that a record
survives. Filesystem aliasing, private namespace integrity and allocation of
fresh handles are host premises. Multiple units may use the SAME entry name. -/
def Op.Away : Op → Fact → Prop
  | .bind l _, .binding k _ => k ≠ l
  | .write n _, .content k _ | .chmod n _, .content k _ => k ≠ n
  | .rename a b _, .binding k _ => k ≠ a ∧ k ≠ b
  | .publish r _, .binding k _ => k ≠ r.tmp ∧ k ≠ r.dst
  | .publish r _, .content k _ => k ≠ r.inode
  | _, _ => True

theorem Op.frame {op : Op} {f : Fact} {s : FS} (h : f.Stable s) (ha : op.Away f) :
    f.Stable (op.exec s) := by
  cases f with
  | binding k n =>
    change StableLink s k (some n) at h
    cases op with
    | bind => exact h.bind ha
    | write => exact h.write
    | chmod => exact h.chmod
    | rename => exact h.rename ha.1 ha.2
    | syncFile => exact h.syncFile
    | syncDir => exact h.syncDir
    | publish r a =>
      cases a with
      | Stage => exact ((h.bind ha.1).bind ha.1).write.chmod
      | SyncFile => exact h.syncFile
      | PublishName => exact h.rename ha.1 ha.2
      | SyncNamespace => exact h.syncDir
      | Complete | Refused => exact h
  | content k data =>
    change StableFile s k data at h
    cases op with
    | bind => exact h.bind
    | write => exact h.write ha
    | chmod => exact h.chmod ha
    | rename => exact h.rename
    | syncFile => exact h.syncFile
    | syncDir => exact h.syncDir
    | publish r a =>
      cases a with
      | Stage => exact ((h.bind.bind).write ha).chmod ha
      | SyncFile => exact h.syncFile
      | PublishName => exact h.rename
      | SyncNamespace => exact h.syncDir
      | Complete | Refused => exact h

theorem exec_frame {s ops f} (h : Fact.Stable f s) (ha : ∀ op ∈ ops, Op.Away op f) :
    f.Stable (execOps s ops) := by
  induction ops generalizing s with
  | nil => exact h
  | cons op rest ih => exact ih (op.frame h (ha op (by simp))) (fun o ho => ha o (by simp [ho]))

/-- The initial parent binding can be merely visible. `create_dir_all` seals
it even when it already exists; a pinned handle by itself would not suffice. -/
inductive Prepare (d : Directory) : FS → FS → Prop where
  | existing {s} : s.visible.link d.loc = some d.inode → Prepare d s (s.syncDir d.loc.parent)
  | created {s} : s.visible.link d.loc = none →
      Prepare d s ((s.bind d.loc (some d.inode)).syncDir d.loc.parent)

theorem Prepare.durable {d s t} (h : Prepare d s t) : d.fact.Stable t := by
  cases h with
  | existing h => exact fun _ hi => (crash_syncDir_link hi rfl).trans h
  | created =>
    intro i hi
    change i.link _ = _
    rw [crash_syncDir_link hi rfl]
    simp [FS.bind, FS.modify, Image.bind]

/-- Wire file priors store a raw hash and original mode. The private blob's
own permissions are distinct. Links and absence are inline in the entry. -/
inductive Prior where
  | absent
  | link (target : String)
  | file (directory : Directory) (blob : Recipe) (originalMode : Nat)

def Prior.facts : Prior → List Fact
  | .file dir blob _ => dir.fact :: recordFacts blob
  | _ => []

/-- Capture cannot answer Done after a partial write, failed sync or failed
recheck. `ops` is the successful path only. `pre` admits directory creation
and quarantine of corrupt existing bytes; its footprint on previously saved
objects is checked by the caller. The subsequent barriers are compulsory. -/
inductive Capture : Prior → FS → List Op → Prop where
  | absent (s) : Capture .absent s []
  | link (s t) : Capture (.link t) s []
  | miss (s : FS) (dir : Directory) (r : Recipe) (mode : Nat) (pre : List Op)
      (parent : (execOps s pre).visible.link dir.loc = some dir.inode)
      (inside : r.dst.parent = dir.inode)
      (fresh : Enabled r .Stage ((execOps s pre).syncDir dir.loc.parent)) :
      Capture (.file dir r mode) s (pre ++ [.syncDir dir.loc.parent, .publish r .Stage,
        .publish r .SyncFile, .publish r .PublishName, .publish r .SyncNamespace])
  | hit (s : FS) (dir : Directory) (r : Recipe) (mode : Nat) (pre : List Op)
      (parent : (execOps s pre).visible.link dir.loc = some dir.inode)
      (inside : r.dst.parent = dir.inode)
      (bound : (execOps s pre).visible.link r.dst = some r.inode)
      (bytes : ((execOps s pre).visible.file r.inode).bytes = r.value.bytes) :
      Capture (.file dir r mode) s (pre ++ [.syncDir dir.loc.parent, .chmod r.inode r.value.mode,
        .syncFile r.inode, .syncDir dir.inode])

/-- The directory name is outside its children. For both capture paths this
separation and a bound parent, not a global fsync, make it reachable. -/
theorem capture_durable {p s ops} (hc : Capture p s ops)
    (separate : ∀ dir r m, p = .file dir r m → dir.loc ≠ r.tmp ∧ dir.loc ≠ r.dst) :
    ∀ f ∈ p.facts, Fact.Stable f (execOps s ops) := by
  cases hc with
  | absent | link => simp [Prior.facts]
  | miss s d r m pre parent inside fresh =>
    let base := (execOps s pre).syncDir d.loc.parent
    have hd : d.fact.Stable base := fun _ hi => (crash_syncDir_link hi rfl).trans parent
    obtain ⟨ht, hn⟩ := separate d r m rfl
    intro f hf
    simp only [Prior.facts, recordFacts, List.mem_cons, List.not_mem_nil, or_false] at hf
    simp only [execOps, List.foldl_append, List.foldl_cons, List.foldl_nil, Op.exec]
    change f.Stable (published r base)
    rcases hf with rfl | rfl | rfl
    · have staged : StableLink (stage r base) d.loc (some d.inode) :=
        (Op.publish r .Stage).frame hd ⟨ht, hn⟩
      exact (staged.syncFile.rename ht hn).syncDir
    · exact fun i hi => (checked_publication_record r base fresh i hi).1
    · exact fun i hi => (checked_publication_record r base fresh i hi).2
  | hit s d r m pre parent inside bound bytes =>
    let base := (execOps s pre).syncDir d.loc.parent
    let privateFile := base.chmod r.inode r.value.mode
    have hd : StableLink base d.loc (some d.inode) := fun _ hi => (crash_syncDir_link hi rfl).trans parent
    have hf : privateFile.visible.file r.inode = r.value := by
      change (if r.inode = r.inode then ⟨((execOps s pre).visible.file r.inode).bytes, r.value.mode⟩ else _) = r.value
      rw [ite_eq_left rfl, bytes]
    intro f h
    simp only [Prior.facts, recordFacts, List.mem_cons, List.not_mem_nil, or_false] at h
    simp only [execOps, List.foldl_append, List.foldl_cons, List.foldl_nil, Op.exec]
    change f.Stable ((privateFile.syncFile r.inode).syncDir d.inode)
    rcases h with rfl | rfl | rfl
    · exact hd.chmod.syncFile.syncDir
    · exact fun _ hi => (crash_syncDir_link hi inside).trans bound
    · exact ((show StableFile (privateFile.syncFile r.inode) r.inode r.value from
        fun _ hi => (crash_syncFile_file hi).trans hf).syncDir)

end Gripsack.Protocol.Leaf
