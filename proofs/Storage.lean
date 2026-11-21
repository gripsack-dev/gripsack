import Mathlib.Logic.Function.Basic

/-!
# Storage at the leaf journal boundary (RFC 0056 §4)

S1 concerns namespace operations, S2 completed barriers, and S3 the two crash
kinds. `FS` denotes the current visible image and alternative power images;
it is not a trace of successful protocol acknowledgements. An ordinary update
can survive independently on **each** old image. A barrier projects precisely
one component of the visible image onto **each** old image. In particular an
fsync is neither a commit of the whole filesystem nor a choice of just one
favourable pre-barrier image.

This is a conditional model for disk-backed Linux filesystems satisfying
these laws. A WSL2 guest filesystem needs the same assumptions. A mounted
Windows volume is not thereby covered (including name equivalence); tmpfs
admits the process-kill interpretation, not reboot persistence. There is no
physical power-cut, syscall availability, termination or liveness theorem.
-/
namespace Gripsack.Storage

/-- A single validated name relative to a directory inode, not a path. -/
structure Loc where
  parent : Nat
  name : Nat
  deriving DecidableEq, Repr

structure FileData where
  bytes : List UInt8
  mode : Nat
  deriving DecidableEq, Repr

/-- FileData is observable only through a regular inode. Values for unallocated
inodes and directories are immaterial. Inode identities do not get recycled. -/
structure Image where
  link : Loc → Option Nat
  file : Nat → FileData

/-- Contents at a location known to name a regular file, or absence. -/
def Image.read (i : Image) (l : Loc) : Option FileData := (i.link l).map i.file

def Image.bind (i : Image) (l : Loc) (v : Option Nat) : Image :=
  { i with link := fun k => if k = l then v else i.link k }

def Image.write (i : Image) (n : Nat) (bytes : List UInt8) : Image :=
  { i with file := fun k => if k = n then { i.file k with bytes := bytes } else i.file k }

def Image.chmod (i : Image) (n mode : Nat) : Image :=
  { i with file := fun k => if k = n then { i.file k with mode := mode } else i.file k }

/-- Both legs of the within-directory rename are a *single* power-loss choice.
On replay, remove the source only if it still names the moved inode; an older
image might contain an unrelated source. The checked operation below handles
two aliases of the same inode as a successful no-op, not a destructive move. -/
def Image.rename (i : Image) (src dst : Loc) (n : Nat) : Image :=
  if i.link src = some n then (i.bind src none).bind dst (some n)
  else i.bind dst (some n)

def Image.copyFile (i visible : Image) (n : Nat) : Image :=
  { i with file := fun k => if k = n then visible.file k else i.file k }

def Image.copyDir (i visible : Image) (d : Nat) : Image :=
  { i with link := fun l => if l.parent = d then visible.link l else i.link l }

structure FS where
  visible : Image
  possible : Image → Prop

/-- Power loss may retain all updates. Kill sees the same visible image but
does *not* resolve the remaining durability choices. -/
def CrashImage (s : FS) (i : Image) : Prop := i = s.visible ∨ s.possible i

def FS.stable (i : Image) : FS := ⟨i, fun j => j = i⟩

def FS.modify (s : FS) (f : Image → Image) : FS :=
  ⟨f s.visible, fun i => ∃ j, CrashImage s j ∧ (i = j ∨ i = f j)⟩

/-- Only used with projections of `s.visible` which fix `s.visible`. -/
def FS.barrier (s : FS) (f : Image → Image) : FS :=
  ⟨s.visible, fun i => ∃ j, CrashImage s j ∧ i = f j⟩

def FS.bind (s : FS) (l : Loc) (v : Option Nat) : FS := s.modify (fun i => i.bind l v)
def FS.write (s : FS) (n : Nat) (bytes : List UInt8) : FS := s.modify (fun i => i.write n bytes)
def FS.chmod (s : FS) (n mode : Nat) : FS := s.modify (fun i => i.chmod n mode)

/-- Visible rename is one syscall in either case. For different parents only,
the two entries can persist independently; no intermediate process view or
driver acknowledgement is inserted between these two power-image choices. -/
def FS.rename (s : FS) (src dst : Loc) (n : Nat) : FS :=
  if src.parent = dst.parent then s.modify (fun i => i.rename src dst n)
  else (s.bind src none).bind dst (some n)

def FS.syncFile (s : FS) (n : Nat) : FS := s.barrier (fun i => i.copyFile s.visible n)
def FS.syncDir (s : FS) (d : Nat) : FS := s.barrier (fun i => i.copyDir s.visible d)

@[simp] theorem crash_stable {i j : Image} : CrashImage (FS.stable i) j ↔ j = i := by
  simp [CrashImage, FS.stable]

theorem crash_visible (s : FS) : CrashImage s s.visible := Or.inl rfl

theorem crash_modify_skip {s f i} (h : CrashImage s i) : CrashImage (s.modify f) i :=
  Or.inr ⟨i, h, Or.inl rfl⟩

theorem crash_modify_take {s f i} (h : CrashImage s i) : CrashImage (s.modify f) (f i) :=
  Or.inr ⟨i, h, Or.inr rfl⟩

theorem crash_modify_iff {s : FS} {f i} : CrashImage (s.modify f) i ↔
    ∃ j, CrashImage s j ∧ (i = j ∨ i = f j) := by
  constructor
  · rintro (rfl | h)
    · exact ⟨s.visible, crash_visible _, Or.inr rfl⟩
    · exact h
  · exact Or.inr

theorem crash_barrier_take {s f i} (h : CrashImage s i) : CrashImage (s.barrier f) (f i) :=
  Or.inr ⟨i, h, rfl⟩

theorem crash_barrier_iff {s : FS} {f : Image → Image} {i} (hfix : f s.visible = s.visible) :
    CrashImage (s.barrier f) i ↔ ∃ j, CrashImage s j ∧ i = f j := by
  constructor
  · rintro (rfl | h)
    · exact ⟨s.visible, crash_visible _, hfix.symm⟩
    · exact h
  · exact Or.inr

@[simp] theorem Image.copyFile_self (i : Image) (n : Nat) : i.copyFile i n = i := by
  simp [Image.copyFile]

@[simp] theorem Image.copyDir_self (i : Image) (d : Nat) : i.copyDir i d = i := by
  simp [Image.copyDir]

theorem crash_syncFile_iff {s : FS} {n i} : CrashImage (s.syncFile n) i ↔
    ∃ j, CrashImage s j ∧ i = j.copyFile s.visible n :=
  crash_barrier_iff (Image.copyFile_self _ _)

theorem crash_syncDir_iff {s : FS} {d i} : CrashImage (s.syncDir d) i ↔
    ∃ j, CrashImage s j ∧ i = j.copyDir s.visible d :=
  crash_barrier_iff (Image.copyDir_self _ _)

theorem crash_syncFile_file {s : FS} {n i} (h : CrashImage (s.syncFile n) i) :
    i.file n = s.visible.file n := by
  rcases crash_syncFile_iff.mp h with ⟨j, _, rfl⟩
  simp [Image.copyFile]

theorem crash_syncDir_link {s : FS} {d i} {l : Loc} (h : CrashImage (s.syncDir d) i) (hd : l.parent = d) :
    i.link l = s.visible.link l := by
  rcases crash_syncDir_iff.mp h with ⟨j, _, rfl⟩
  simp [Image.copyDir, hd]

theorem crash_syncDir_files {s : FS} {d i} (h : CrashImage (s.syncDir d) i) :
    ∃ j, CrashImage s j ∧ i.file = j.file := by
  rcases crash_syncDir_iff.mp h with ⟨j, hj, rfl⟩
  exact ⟨j, hj, rfl⟩

theorem crash_syncFile_links {s : FS} {n i} (h : CrashImage (s.syncFile n) i) :
    ∃ j, CrashImage s j ∧ i.link = j.link := by
  rcases crash_syncFile_iff.mp h with ⟨j, hj, rfl⟩
  exact ⟨j, hj, rfl⟩

@[simp] theorem Image.rename_files (i : Image) (src dst : Loc) (n : Nat) :
    (i.rename src dst n).file = i.file := by
  unfold Image.rename
  split <;> rfl

theorem crash_modify_frame {α : Sort _} (observe : Image → α) {s : FS} {f : Image → Image} {i}
    (hframe : ∀ j, observe (f j) = observe j) (h : CrashImage (s.modify f) i) :
    ∃ j, CrashImage s j ∧ observe i = observe j := by
  rcases crash_modify_iff.mp h with ⟨j, hj, rfl | rfl⟩
  · exact ⟨i, hj, rfl⟩
  · exact ⟨j, hj, hframe j⟩

theorem crash_bind_files {s : FS} {l v i} (h : CrashImage (s.bind l v) i) :
    ∃ j, CrashImage s j ∧ i.file = j.file :=
  crash_modify_frame Image.file (f := fun j => j.bind l v) (fun _ => rfl) h

theorem crash_rename_files {s : FS} {src dst n i} (h : CrashImage (s.rename src dst n) i) :
    ∃ j, CrashImage s j ∧ i.file = j.file := by
  unfold FS.rename at h
  split at h
  · exact crash_modify_frame Image.file (fun j => j.rename_files src dst n) h
  · obtain ⟨k, hk, e₁⟩ := crash_bind_files h
    obtain ⟨j, hj, e₂⟩ := crash_bind_files hk
    exact ⟨j, hj, e₁.trans e₂⟩

/-- The exact two choices, including both source and destination; there is
no independent per-leg survival choice for a rename in one directory. -/
theorem withinDir_rename_atomic {s : FS} {src dst : Loc} {n i} (hd : src.parent = dst.parent) :
    CrashImage (s.rename src dst n) i ↔ ∃ j, CrashImage s j ∧ (i = j ∨ i = j.rename src dst n) := by
  simpa [FS.rename, hd] using
    (crash_modify_iff (s := s) (f := fun j => j.rename src dst n) (i := i))

@[simp] theorem Image.read_bind_self (i : Image) (l : Loc) (n : Nat) :
    (i.bind l (some n)).read l = some (i.file n) := by simp [Image.read, Image.bind]

@[simp] theorem Image.read_unlink_self (i : Image) (l : Loc) : (i.bind l none).read l = none := by
  simp [Image.read, Image.bind]

theorem Image.read_bind_other {i : Image} {l k : Loc} {v} (h : k ≠ l) :
    (i.bind l v).read k = i.read k := by simp [Image.read, Image.bind, h]

theorem Image.read_write {i : Image} {l : Loc} {n : Nat} (h : i.link l = some n) (b : List UInt8) :
    (i.write n b).read l = some { i.file n with bytes := b } := by simp [Image.read, Image.write, h]

theorem Image.read_chmod {i : Image} {l : Loc} {n : Nat} (h : i.link l = some n) (m : Nat) :
    (i.chmod n m).read l = some { i.file n with mode := m } := by simp [Image.read, Image.chmod, h]

theorem Image.rename_moves {i : Image} {src dst : Loc} {n : Nat}
    (hne : src ≠ dst) (hsrc : i.link src = some n) :
    (i.rename src dst n).link src = none ∧ (i.rename src dst n).link dst = some n := by
  simp [Image.rename, hsrc, Image.bind, hne]

@[simp] theorem visible_rename_dst (s : FS) (src dst : Loc) (n : Nat) :
    (s.rename src dst n).visible.link dst = some n := by
  by_cases hd : src.parent = dst.parent <;> by_cases hsrc : s.visible.link src = some n <;>
    simp [FS.rename, hd, FS.modify, FS.bind, Image.rename, Image.bind, hsrc]

def All (s : FS) (p : Image → Prop) : Prop := ∀ i, CrashImage s i → p i
def StableLink (s : FS) (l : Loc) (v : Option Nat) : Prop := All s (fun i => i.link l = v)
def StableFile (s : FS) (n : Nat) (v : FileData) : Prop := All s (fun i => i.file n = v)

theorem All.modify {s p} (h : All s p) {f} (hf : ∀ i, p i → p (f i)) : All (s.modify f) p := by
  intro i hi
  rcases crash_modify_iff.mp hi with ⟨j, hj, rfl | rfl⟩
  · exact h i hj
  · exact hf j (h j hj)

section frames
variable {s : FS} {k l src dst : Loc} {n a m d : Nat} {b : List UInt8} {v w : Option Nat} {x : FileData}

theorem StableLink.visible (h : StableLink s k v) : s.visible.link k = v := h _ (crash_visible _)
theorem StableFile.visible (h : StableFile s a x) : s.visible.file a = x := h _ (crash_visible _)

theorem StableFile.after_syncFile (s : FS) (n : Nat) : StableFile (s.syncFile n) n (s.visible.file n) :=
  fun _ hi => crash_syncFile_file hi
theorem StableLink.after_syncDir (s : FS) (k : Loc) : StableLink (s.syncDir k.parent) k (s.visible.link k) :=
  fun _ hi => crash_syncDir_link hi rfl

theorem StableLink.bind (h : StableLink s k v) (hk : k ≠ l) : StableLink (s.bind l w) k v := by
  apply All.modify h
  intro i hi
  simpa [Image.bind, hk] using hi

theorem StableLink.write (h : StableLink s k v) : StableLink (s.write n b) k v :=
  All.modify h (fun _ hi => hi)
theorem StableLink.chmod (h : StableLink s k v) : StableLink (s.chmod n m) k v :=
  All.modify h (fun _ hi => hi)

theorem StableLink.rename (h : StableLink s k v) (hs : k ≠ src) (ht : k ≠ dst) :
    StableLink (s.rename src dst n) k v := by
  unfold FS.rename
  split
  · apply All.modify h
    intro i hi
    unfold Image.rename
    split <;> simpa [Image.bind, hs, ht] using hi
  · exact (h.bind hs).bind ht

theorem StableLink.syncDir (h : StableLink s k v) : StableLink (s.syncDir d) k v := by
  intro i hi
  rcases crash_syncDir_iff.mp hi with ⟨j, hj, rfl⟩
  by_cases e : k.parent = d <;> simp [Image.copyDir, e, h.visible, h j hj]
theorem StableLink.syncFile (h : StableLink s k v) : StableLink (s.syncFile n) k v := by
  intro i hi
  rcases crash_syncFile_iff.mp hi with ⟨j, hj, rfl⟩
  exact h j hj

theorem StableFile.bind (h : StableFile s a x) : StableFile (s.bind l v) a x :=
  All.modify h (fun _ hi => hi)
theorem StableFile.write (h : StableFile s a x) (hk : a ≠ n) : StableFile (s.write n b) a x := by
  apply All.modify h
  intro i hi
  simpa [Image.write, hk] using hi
theorem StableFile.chmod (h : StableFile s a x) (hk : a ≠ n) : StableFile (s.chmod n m) a x := by
  apply All.modify h
  intro i hi
  simpa [Image.chmod, hk] using hi
theorem StableFile.rename (h : StableFile s a x) : StableFile (s.rename src dst n) a x := by
  intro i hi
  obtain ⟨j, hj, e⟩ := crash_rename_files hi
  rw [e]
  exact h j hj
theorem StableFile.syncDir (h : StableFile s a x) : StableFile (s.syncDir d) a x := by
  intro i hi
  rcases crash_syncDir_iff.mp hi with ⟨j, hj, rfl⟩
  exact h j hj
theorem StableFile.syncFile (h : StableFile s a x) : StableFile (s.syncFile n) a x := by
  intro i hi
  rcases crash_syncFile_iff.mp hi with ⟨j, hj, rfl⟩
  change (if a = n then s.visible.file a else j.file a) = x
  split
  · exact h.visible
  · exact h j hj
end frames

/-! ## Checked host commands

`FS` transformers describe effects, not permissions or successful syscalls.
`Machine` adds the (monotonically allocated) inode kinds and the current
process's descriptor table. Allocation chooses a never-used identity; its
initial contents are meaningful only once a surviving name points at it.
A handle pins an inode, *not* the name used to open it. In particular a
replacing rename does not retarget already-open handles. Directory fds are
identified by the directory they pin; distinct fds for the same directory
have identical authority in this fragment.
-/

inductive Kind where
  | regular
  | directory
  | symlink (target : List UInt8)
  | other
  deriving DecidableEq

structure Handle where
  ino : Nat
  offset : Nat
  writable : Bool

structure Session where
  dirs : Nat → Prop
  files : Nat → Option Handle

def Session.root (n : Nat) : Session := ⟨fun k => k = n, fun _ => none⟩
def Session.pin (p : Session) (n : Nat) : Session := { p with dirs := fun k => k = n ∨ p.dirs k }
def Session.put (p : Session) (slot : Nat) (h : Handle) : Session :=
  { p with files := fun k => if k = slot then some h else p.files k }

structure Machine where
  fs : FS
  kind : Nat → Option Kind
  root : Nat
  session : Option Session

def Machine.live (m : Machine) (p : Session) : Machine := { m with session := some p }
def Machine.kill (m : Machine) : Machine := { m with session := none }
def Machine.power (m : Machine) (i : Image) : Machine := { m.kill with fs := FS.stable i }

inductive CrashKind where | kill | power
inductive Crash (m : Machine) : CrashKind → Machine → Prop where
  | kill : Crash m .kill m.kill
  | power {i} (h : CrashImage m.fs i) : Crash m .power (m.power i)

def PinnedDir (m : Machine) (p : Session) (d : Nat) : Prop :=
  m.session = some p ∧ p.dirs d ∧ m.kind d = some .directory

def PinnedFile (m : Machine) (p : Session) (slot : Nat) (h : Handle) : Prop :=
  m.session = some p ∧ p.files slot = some h ∧ m.kind h.ino = some .regular

/-- The allocator does not hand out an inode still denoted by *any* old
image. A new directory starts empty, including on a later power cut. -/
def Fresh (m : Machine) (n : Nat) : Prop :=
  m.kind n = none ∧ All m.fs (fun i => ∀ l, i.link l ≠ some n ∧ (l.parent = n → i.link l = none))

def Image.initFile (i : Image) (n : Nat) (mode : Nat) : Image :=
  { i with file := fun k => if k = n then ⟨[], mode⟩ else i.file k }

def FS.initFile (s : FS) (n mode : Nat) : FS :=
  ⟨s.visible.initFile n mode, fun i => ∃ j, CrashImage s j ∧ i = j.initFile n mode⟩

def Machine.allocate (m : Machine) (l : Loc) (n : Nat) (kind : Kind) (s : FS) : Machine :=
  { m with fs := s.bind l (some n), kind := fun k => if k = n then some kind else m.kind k }

/-- A directory cannot be moved underneath itself. No claim about the
protocol for evolving a receipt's directory tree (premise D) is made here. -/
inductive Below (m : Machine) (root : Nat) : Nat → Prop where
  | self : Below m root root
  | entry {d n name} (hd : Below m root d) (h : m.fs.visible.link ⟨d, name⟩ = some n)
      (hk : m.kind n = some .directory) : Below m root n

def DirEmpty (m : Machine) (d : Nat) : Prop := ∀ name, m.fs.visible.link ⟨d, name⟩ = none

/-- For distinct source and target identities, POSIX kind/emptiness
conditions and `RENAME_NOREPLACE` are checked in the syscall's current view.
There is **no** comparison against a digest or earlier application snapshot. -/
def CanReplace (m : Machine) (srcKind : Kind) (dst : Loc) (replace : Bool) : Prop :=
  match m.fs.visible.link dst with
  | none => True
  | some n => replace = true ∧ ∃ k, m.kind n = some k ∧
      ((srcKind = .directory ∧ k = .directory ∧ DirEmpty m n) ∨
       (srcKind ≠ .directory ∧ k ≠ .directory))

/-! `RealFs::write` loops. Each successful syscall writes its byte range at the
open handle's offset; if only a later range persists, the intervening gap reads
as zeros. A successful *call* can comprise several independently surviving
ranges. `FS.write` above is the whole-content assignment used for a single
completed fresh-file fill, not a specification that all write-all loops are
atomic. Mode is independent throughout. -/
def Image.writeAt (i : Image) (n off : Nat) (chunk : List UInt8) : Image :=
  let old := (i.file n).bytes
  let bytes := if old.length ≤ off then old ++ List.replicate (off - old.length) 0 ++ chunk
               else old.take off ++ chunk ++ old.drop (off + chunk.length)
  i.write n bytes

def FS.writeAt (s : FS) (n off : Nat) (chunk : List UInt8) : FS :=
  s.modify (fun i => i.writeAt n off chunk)

inductive Chunks (n : Nat) : Nat → List UInt8 → FS → FS → Prop where
  | nil {off s} : Chunks n off [] s s
  | cons {off} {s t : FS} {chunk rest} (positive : chunk ≠ [])
      (more : Chunks n (off + chunk.length) rest (s.writeAt n off chunk) t) :
      Chunks n off (chunk ++ rest) s t

theorem Chunks.one {s : FS} {n off : Nat} {b : List UInt8} (hb : b ≠ []) :
    Chunks n off b s (s.writeAt n off b) := by
  simpa using (Chunks.cons (n := n) (s := s) (off := off) (rest := []) hb Chunks.nil)

/-- Every successful chunking on a fresh/append-position handle produces the
same visible bytes, without identifying the pre-barrier power images. -/
theorem Chunks.appended {n off b s t} (run : Chunks n off b s t)
    (atEnd : off = (s.visible.file n).bytes.length) :
    (t.visible.file n).bytes = (s.visible.file n).bytes ++ b := by
  induction run with
  | nil => simp
  | @cons off s t chunk rest _ _ ih =>
    have one : ((s.writeAt n off chunk).visible.file n).bytes = (s.visible.file n).bytes ++ chunk := by
      simp [FS.writeAt, FS.modify, Image.writeAt, Image.write, atEnd]
    have nextPos : off + chunk.length = ((s.writeAt n off chunk).visible.file n).bytes.length := by
      rw [one, List.length_append, atEnd]
    rw [ih nextPos, one, List.append_assoc]

theorem Chunks.stableLink {n off b s t} (run : Chunks n off b s t) {l v} :
    StableLink s l v → StableLink t l v := by
  induction run with
  | nil => exact id
  | cons _ _ ih => exact fun h => ih (All.modify h (fun _ hj => hj))

inductive Status where | ok | error | killed
  deriving DecidableEq

/-- Bytes that the write-all loop actually transferred, including an ordinary
I/O error or death following an arbitrary prefix. The suffix may be empty
if death occurs after the last syscall but before acknowledgement. -/
inductive WriteExtent : List UInt8 → List UInt8 → Status → Prop where
  | complete (b) : WriteExtent b b .ok
  | failed (part tail) : WriteExtent (part ++ tail) part .error
  | interrupted (part tail) : WriteExtent (part ++ tail) part .killed

theorem partial_write_never_acknowledges {wanted actual r} (h : WriteExtent wanted actual r)
    (short : actual.length < wanted.length) : r ≠ .ok := by
  cases h <;> simp_all

def Machine.afterWrite (m : Machine) (p : Session) (slot : Nat) (h : Handle)
    (s : FS) (count : Nat) (r : Status) : Machine :=
  { m with fs := s, session := match r with
      | .killed => none
      | _ => some (p.put slot { h with offset := h.offset + count }) }

/-- `createFile` takes the *effective* creation mode (after host umask), not
a promise that the host honored every requested permission bit. Exact final
mode is established by fchmod. Descriptor slots identify return values.
Observations carry their actual result, so a successful read of another
inode cannot be used as evidence about this one. -/
inductive Command where
  | acquire
  | release
  | openDir (l : Loc)
  | createDir (l : Loc)
  | openFile (l : Loc) (slot : Nat)
  | createFile (l : Loc) (slot mode : Nat)
  | readFile (slot : Nat) (result : List UInt8)
  | stat (l : Loc) (result : Option (Nat × Kind))
  | readLink (l : Loc) (result : List UInt8)
  | symlink (l : Loc) (target : List UInt8)
  | writeAll (slot : Nat) (bytes : List UInt8)
  | chmod (slot mode : Nat)
  | syncFile (slot : Nat)
  | syncDir (d : Nat)
  | rename (src dst : Loc) (replace : Bool)
  | unlink (l : Loc)
  | rmdir (l : Loc)

/-- Even a pinned directory which was unlinked cannot gain fresh entries. -/
def LiveDir (m : Machine) (d : Nat) : Prop :=
  d = m.root ∨ ∃ l, m.fs.visible.link l = some d

/-- Completed, successful non-write effects. (Write success and post-prefix
non-success share the checked `Step.writeAll` rule below.) Permissions,
read-only media, space limits and I/O errors may always prevent a success;
the model does not predict syscall availability. A name operation always
takes the checked, pinned parent. -/
inductive Completed (m : Machine) : Command → Machine → Prop where
  | acquire (free : m.session = none) (dir : m.kind m.root = some .directory) :
      Completed m .acquire (m.live (Session.root m.root))
  | release {p} (held : m.session = some p) : Completed m .release m.kill
  | openDir {p} {l : Loc} {n} (dir : PinnedDir m p l.parent) (found : m.fs.visible.link l = some n)
      (kind : m.kind n = some .directory) : Completed m (.openDir l) (m.live (p.pin n))
  | createDir {p} {l : Loc} {n} (dir : PinnedDir m p l.parent) (live : LiveDir m l.parent)
      (free : m.fs.visible.link l = none)
      (fresh : Fresh m n) : Completed m (.createDir l) (m.allocate l n .directory m.fs)
  | openFile {p} {l : Loc} {n slot} (dir : PinnedDir m p l.parent) (found : m.fs.visible.link l = some n)
      (kind : m.kind n = some .regular) (unused : p.files slot = none) :
      Completed m (.openFile l slot) (m.live (p.put slot ⟨n, 0, false⟩))
  | createFile {p} {l : Loc} {n slot mode} (dir : PinnedDir m p l.parent) (live : LiveDir m l.parent)
      (free : m.fs.visible.link l = none)
      (fresh : Fresh m n) (unused : p.files slot = none) :
      Completed m (.createFile l slot mode)
        ((m.allocate l n .regular (m.fs.initFile n mode)).live (p.put slot ⟨n, 0, true⟩))
  | readFile {p slot h} (fd : PinnedFile m p slot h) :
      Completed m (.readFile slot (m.fs.visible.file h.ino).bytes) m
  | stat {p} {l : Loc} {n k} (dir : PinnedDir m p l.parent) (found : m.fs.visible.link l = some n)
      (kind : m.kind n = some k) : Completed m (.stat l (some (n,k))) m
  | absent {p} {l : Loc} (dir : PinnedDir m p l.parent) (missing : m.fs.visible.link l = none) :
      Completed m (.stat l none) m
  | readLink {p} {l : Loc} {n target} (dir : PinnedDir m p l.parent) (found : m.fs.visible.link l = some n)
      (kind : m.kind n = some (.symlink target)) : Completed m (.readLink l target) m
  | symlink {p} {l : Loc} {n target} (dir : PinnedDir m p l.parent) (live : LiveDir m l.parent)
      (free : m.fs.visible.link l = none)
      (fresh : Fresh m n) : Completed m (.symlink l target) (m.allocate l n (.symlink target) m.fs)
  | chmod {p slot h mode} (fd : PinnedFile m p slot h) :
      Completed m (.chmod slot mode) { m with fs := m.fs.chmod h.ino mode }
  | syncFile {p slot h} (fd : PinnedFile m p slot h) :
      Completed m (.syncFile slot) { m with fs := m.fs.syncFile h.ino }
  | syncDir {p d} (dir : PinnedDir m p d) : Completed m (.syncDir d) { m with fs := m.fs.syncDir d }
  | rename {p} {src dst : Loc} {n k replace} (source : PinnedDir m p src.parent)
      (target : PinnedDir m p dst.parent) (live : LiveDir m dst.parent)
      (found : m.fs.visible.link src = some n) (kind : m.kind n = some k)
      (distinct : m.fs.visible.link dst ≠ some n) (accept : CanReplace m k dst replace)
      (acyclic : k = .directory → ¬ Below m n dst.parent) :
      Completed m (.rename src dst replace) { m with fs := m.fs.rename src dst n }
  | renameAlias {p} {src dst : Loc} {n} (source : PinnedDir m p src.parent) (target : PinnedDir m p dst.parent)
      (found : m.fs.visible.link src = some n) (same : m.fs.visible.link dst = some n) :
      Completed m (.rename src dst true) m
  | unlink {p} {l : Loc} {n k} (dir : PinnedDir m p l.parent) (found : m.fs.visible.link l = some n)
      (kind : m.kind n = some k) (notDir : k ≠ .directory) :
      Completed m (.unlink l) { m with fs := m.fs.bind l none }
  | rmdir {p} {l : Loc} {n} (dir : PinnedDir m p l.parent) (found : m.fs.visible.link l = some n)
      (kind : m.kind n = some .directory) (empty : DirEmpty m n) :
      Completed m (.rmdir l) { m with fs := m.fs.bind l none }

inductive Step (m : Machine) (c : Command) : Status → Machine → Prop where
  | completed {t} (exec : Completed m c t) : Step m c .ok t
  | errorBefore : Step m c .error m
  /-- In particular, a reported fsync error is not proof that none of its
  writes reached disk. Even a completed effect with an error is not `.ok`. -/
  | errorAfter {t} (exec : Completed m c t) : Step m c .error t
  | killedBefore : Step m c .killed m.kill
  | lostReply {t} (exec : Completed m c t) : Step m c .killed t.kill
  | writeAll {p slot h wanted actual s r} (call : c = .writeAll slot wanted)
      (fd : PinnedFile m p slot h) (w : h.writable = true) (extent : WriteExtent wanted actual r)
      (writes : Chunks h.ino h.offset actual m.fs s) :
      Step m c r (m.afterWrite p slot h s actual.length r)

/-- Recursive removal is a sequence of checked file unlinks / empty-directory
rmdirs. Every intermediate machine is a crash point; there is no `removeTree`
atomic effect or implicit fsync. Refusal stops at the last completed step. -/
inductive Unlinks : Machine → List Loc → Machine → Prop where
  | nil {m} : Unlinks m [] m
  | cons {m l t rest u}
      (one : Completed m (.unlink l) t ∨ Completed m (.rmdir l) t)
      (more : Unlinks t rest u) : Unlinks m (l :: rest) u

/-- A single accepted positive-length prefix followed by either I/O error or
death. The former leaves the advanced writer, the latter drops the session;
neither is an acknowledgement. Both have the same bytes in the page cache. -/
theorem post_prefix_outcomes {m : Machine} {p slot h} (fd : PinnedFile m p slot h)
    (w : h.writable = true) (part tail : List UInt8) (nonempty : part ≠ []) :
    let s := m.fs.writeAt h.ino h.offset part
    let call := Command.writeAll slot (part ++ tail)
    Step m call .error (m.afterWrite p slot h s part.length .error) ∧
    Step m call .killed (m.afterWrite p slot h s part.length .killed) := by
  exact ⟨.writeAll rfl fd w (.failed part tail) (Chunks.one nonempty),
         .writeAll rfl fd w (.interrupted part tail) (Chunks.one nonempty)⟩

theorem CanReplace.noreplace_vacant {m k dst} (ok : CanReplace m k dst false) :
    m.fs.visible.link dst = none := by
  cases e : m.fs.visible.link dst with
  | none => rfl
  | some n => simp [CanReplace, e] at ok

theorem noreplace_requires_vacant {m src dst t} (success : Completed m (.rename src dst false) t) :
    m.fs.visible.link dst = none := by
  cases success
  apply CanReplace.noreplace_vacant
  assumption


/-! ## External premises, not storage axioms

S4 excludes a second *lifecycle* writer, not another program. S5 explicitly
allows external writes: a recheck is an observation; its agreement with the
state at the later name operation is a conditional premise, not provided by
opening the parent fd. `Completed.rename` has no compare-and-swap guard.
Premise D about delegated directory steps (journal before changing the
receipt's namespace, settlement/refusal and compensation) remains the
separate directory-domain contract in `Kernel.Directory`, not a law of fsync.
-/

/-- At a given instant at most one process holds the lifecycle lock. -/
def S4Serialized (holds : Nat → Nat → Prop) : Prop :=
  ∀ time a b, holds time a → holds time b → a = b

/-- The last recheck still describes this name and (for a regular file)
its contents/mode at the act. This is *not* guaranteed in the S5 window.
Inode kind and a symlink's target are immutable for that allocated identity. -/
def S5RecheckStillValid (check act : Image) (l : Loc) : Prop :=
  check.link l = act.link l ∧ check.read l = act.read l

/-- Host names that can address the same entry must compare equivalent in
the kernel. `Loc.name` is already a representative of the host equivalence. -/
def S6NameEquivalence {Name : Type} (hostEq keyEq : Name → Name → Prop) : Prop :=
  ∀ a b, hostEq a b → keyEq a b

/-- Collision-resistance idealization restricted to the byte strings in the
run; not a theorem of unbounded SHA-256 injectivity. -/
def S7DigestIdentity {Digest : Type} (hash : List UInt8 → Digest) (used : List UInt8 → Prop) : Prop :=
  ∀ a b, used a → used b → hash a = hash b → a = b

/-! ## Concrete cuts and negative controls -/

def emptyImage : Image := ⟨fun _ => none, fun _ => ⟨[], 0⟩⟩
private def first : Loc := ⟨0, 10⟩
private def second : Loc := ⟨0, 11⟩
private def elsewhere : Loc := ⟨1, 12⟩

/-- Lose the first bind, retain the second. The two observations together
differ from *every* prefix (empty, first only, or both). -/
theorem power_nonprefix_example :
    ∃ i, CrashImage (((FS.stable emptyImage).bind first (some 2)).bind second (some 3)) i ∧
      i.link first = none ∧ i.link second = some 3 := by
  refine ⟨emptyImage.bind second (some 3), ?_, rfl, rfl⟩
  exact crash_modify_take (s := (FS.stable emptyImage).bind first (some 2))
    (f := fun i => i.bind second (some 3)) (crash_modify_skip (crash_visible (FS.stable emptyImage)))

theorem dir_barrier_excludes_example :
    let s := (FS.stable emptyImage).bind first (some 2)
    CrashImage s emptyImage ∧ ¬ CrashImage (s.syncDir first.parent) emptyImage := by
  dsimp only
  refine ⟨crash_modify_skip (crash_visible (FS.stable emptyImage)), ?_⟩
  intro h
  have eq := crash_syncDir_link (l := first) h rfl
  change (none : Option Nat) = some 2 at eq
  cases eq

/-- With different parents a destination-only sync can leave the old source
present; a source-only sync can leave the new destination absent. -/
theorem crossDir_one_barrier_insufficient :
    let initial := emptyImage.bind first (some 2)
    let moved := (FS.stable initial).rename first elsewhere 2
    (∃ i, CrashImage (moved.syncDir elsewhere.parent) i ∧ i.link first = some 2 ∧ i.link elsewhere = some 2) ∧
    (∃ i, CrashImage (moved.syncDir first.parent) i ∧ i.link first = none ∧ i.link elsewhere = none) := by
  let initial := emptyImage.bind first (some 2)
  let moved := (FS.stable initial).rename first elsewhere 2
  have missed : CrashImage moved initial :=
    crash_modify_skip (s := (FS.stable initial).bind first none) (f := fun i => i.bind elsewhere (some 2))
      (crash_modify_skip (crash_visible (FS.stable initial)))
  constructor
  · refine ⟨initial.copyDir moved.visible elsewhere.parent, ?_, rfl, rfl⟩
    exact crash_barrier_take (f := fun i => i.copyDir moved.visible elsewhere.parent) missed
  · refine ⟨initial.copyDir moved.visible first.parent, ?_, rfl, rfl⟩
    exact crash_barrier_take (f := fun i => i.copyDir moved.visible first.parent) missed

/-- A concrete live writer with an unbarriered name dies. A reader can
observe the new name after kill, yet a *later* power loss may still drop it. -/
theorem kill_example :
    ∃ m t u : Machine, m.session ≠ none ∧ Crash m .kill t ∧ t.session = none ∧
      t.fs.visible.link first = some 2 ∧ Crash t .power u ∧ u.fs.visible.link first = none := by
  let fs := (FS.stable emptyImage).bind first (some 2)
  let m : Machine := ⟨fs, (fun n => if n = 0 then some .directory else some .regular),
    0, some (Session.root 0)⟩
  refine ⟨m, m.kill, m.kill.power emptyImage, ?_, .kill, rfl, rfl, .power ?_, rfl⟩
  · intro e
    change some (Session.root 0) = none at e
    cases e
  · exact crash_modify_skip (crash_visible (FS.stable emptyImage))

end Gripsack.Storage
