import Extracted.GripsackKernel
import Storage
import Tauten.Machine

/-!
# Completing one staged record

`effect` interprets the four successful calls in `RunDriver::publish`. It is
not a syscall success oracle. `Performed` checks the request before running
it; its receipt is the *only* event delivered to the extracted successor.
The terminal theorem is local to the cut after this record's namespace
barrier, not a theorem about recovery or an unverified filesystem/codec.
-/
namespace Gripsack.Protocol.Publication

open Gripsack.Storage gripsack_kernel Aeneas Aeneas.Std Result
open Tauten.Machine

abbrev Action := journal.PublicationAction

structure Recipe where
  tmp : Loc
  dst : Loc
  inode : Nat
  value : FileData

/-- `create_replacing`: discard a stale staging name, exclusively bind a fresh
inode, write the bytes through the pinned handle, set the requested mode. -/
def stage (r : Recipe) (s : FS) : FS :=
  (((s.bind r.tmp none).bind r.tmp (some r.inode)).write r.inode r.value.bytes).chmod
    r.inode r.value.mode

/-- The successful transformer only. Terminal actions cannot be performed. -/
def effect (r : Recipe) (a : Action) (s : FS) : FS :=
  match a with
  | .Stage => stage r s
  | .SyncFile => s.syncFile r.inode
  | .PublishName => s.rename r.tmp r.dst r.inode
  | .SyncNamespace => s.syncDir r.dst.parent
  | .Complete | .Refused => s

def sealed (r : Recipe) (s : FS) : FS := effect r .SyncFile (stage r s)
def renamed (r : Recipe) (s : FS) : FS := effect r .PublishName (sealed r s)
def published (r : Recipe) (s : FS) : FS := effect r .SyncNamespace (renamed r s)

/-- The two barriers settle different components. Neither an old destination
nor an assumption about the content of the unallocated inode is needed. -/
theorem published_record (r : Recipe) (s : FS) (i : Image) (h : CrashImage (published r s) i) :
    i.link r.dst = some r.inode ∧ i.file r.inode = r.value := by
  constructor
  · exact (crash_syncDir_link h (l := r.dst) rfl).trans (visible_rename_dst _ _ _ _)
  · obtain ⟨j, hj, hfile⟩ := crash_syncDir_files h
    obtain ⟨k, hk, hkeep⟩ := crash_rename_files hj
    rw [hfile, hkeep, crash_syncFile_file hk]
    simp [stage, FS.chmod, FS.write, FS.modify, Image.chmod, Image.write]

/-- Same-directory, distinct names; the host allocator never recycles an
inode observable in an admissible image. No restriction on the old target. -/
def Enabled (r : Recipe) (a : Action) (s : FS) : Prop :=
  match a with
  | .Stage => r.tmp.parent = r.dst.parent ∧ r.tmp ≠ r.dst ∧
      ∀ i, CrashImage s i → ∀ l, i.link l ≠ some r.inode
  | .SyncFile | .PublishName => s.visible.link r.tmp = some r.inode
  | .SyncNamespace => s.visible.link r.dst = some r.inode
  | .Complete | .Refused => False

/-- Interrupted staging includes a partially written new file. Other calls
can fail before or after a visible effect; neither outcome produces a receipt. -/
def PartialEffect (r : Recipe) (a : Action) (s p : FS) : Prop :=
  p = s ∨ p = effect r a s ∨ (a = .Stage ∧
    (p = s.bind r.tmp none ∨ p = (s.bind r.tmp none).bind r.tmp (some r.inode) ∨
      ∃ k, p = ((s.bind r.tmp none).bind r.tmp (some r.inode)).write r.inode (r.value.bytes.take k)))

inductive Session where
  | waiting (a : Action) (s : FS)
  | receipt (a : Action) (s : FS)
  | stopped (i : Image)

/-- Moving from `waiting` to `receipt` is the successful host operation. A
failed operation or crash ends this writer session and releases no event. -/
inductive Performed (r : Recipe) : Session → Session → Prop where
  | success {a s} : Enabled r a s →
      Performed r (.waiting a s) (.receipt a (effect r a s))
  | interrupt {a s p i} : PartialEffect r a s p → CrashImage p i →
      Performed r (.waiting a s) (.stopped i)
  | lostReceipt {a s i} : CrashImage s i → Performed r (.receipt a s) (.stopped i)

def world (r : Recipe) : World Unit Action Session where
  deliver v _ := match v with | .receipt _ _ => True | _ => False
  interpret v next v' := match v with
    | .receipt _ s => v' = .waiting next s
    | _ => False
  adversary := Performed r

/-- This is the same translated successor called by `journal_run.run_step`.
The command announces its next request; it never performs the previous one. -/
def extracted : RMachine Action Unit Action where
  step a _ := do
    let next ← journal.PublicationAction.successor a
    ok (next, next)

def next : Action → Action
  | .Stage => .SyncFile
  | .SyncFile => .PublishName
  | .PublishName => .SyncNamespace
  | .SyncNamespace => .Complete
  | .Complete | .Refused => .Refused

def model : Machine Action Unit Action := ⟨fun a _ => (next a, next a)⟩

theorem extracted_agrees (a : Action) (e : Unit) : extracted.step a e = ok (model.step a e) := by
  cases a <;> simp [extracted, model, next, journal.PublicationAction.successor]

def initial (base : FS) (x : Action × Session) : Prop := x = (.Stage, .waiting .Stage base)
abbrev Reach (r : Recipe) (base : FS) := RSysReachable extracted (world r) (initial base)

private def before (r : Recipe) (base : FS) : Action → FS
  | .Stage => base
  | .SyncFile => stage r base
  | .PublishName => sealed r base
  | .SyncNamespace => renamed r base
  | .Complete => published r base
  | .Refused => base

private theorem next_before (r : Recipe) (base : FS) (a : Action)
    (h : Enabled r a (before r base a)) : effect r a (before r base a) = before r base (next a) := by
  cases a <;> first | rfl | exact False.elim h

private def Invariant (r : Recipe) (base : FS) (x : Action × Session) : Prop :=
  match x.2 with
  | .waiting k s => k = x.1 ∧ s = before r base x.1
  | .receipt k s => k = x.1 ∧ Enabled r k (before r base k) ∧ s = effect r k (before r base k)
  | .stopped _ => True

/-- Reachable induction ties the pending request, receipt, and actual FS to
one another. Deliverability alone does not establish this invariant. -/
private theorem reachable_inv (r : Recipe) (base : FS) (x : Action × Session) (h : Reach r base x) :
    Invariant r base x := by
  refine sys_inv_of_step model (world r) (initial base) (Invariant r base) ?_ ?_ ?_ x
    (rsys_of_model extracted model extracted_agrees (world r) (initial base) x h)
  · rintro _ rfl
    exact ⟨rfl, rfl⟩
  · intro a v v' e hv hd hi
    cases v with
    | waiting => exact False.elim hd
    | stopped => exact False.elim hd
    | receipt k s =>
      change v' = .waiting (next a) s at hi
      subst v'
      obtain ⟨hk, hen, hs⟩ := hv
      change k = a at hk
      subst k
      exact ⟨rfl, hs.trans (next_before r base a hen)⟩
  · intro a v v' hv ha
    cases ha with
    | @success k s hen =>
      change k = a ∧ s = before r base a at hv
      obtain ⟨rfl, rfl⟩ := hv
      exact ⟨rfl, hen, rfl⟩
    | interrupt => trivial
    | lostReceipt => trivial

/-- Any completion of the real successor against this world has the right
name, inode, bytes and mode after kill or any admissible power loss. -/
theorem reachable_record (r : Recipe) (base s : FS) (reach : Reach r base (.Complete, .waiting .Complete s))
    (i : Image) (cut : CrashImage s i) : i.link r.dst = some r.inode ∧ i.file r.inode = r.value := by
  obtain ⟨_, hs⟩ := reachable_inv r base _ reach
  change s = published r base at hs
  subst s
  exact published_record r base i cut

private theorem advance (r : Recipe) (base : FS) (a : Action) (s : FS)
    (reach : Reach r base (a, .waiting a s)) (en : Enabled r a s) :
    Reach r base (next a, .waiting (next a) (effect r a s)) := by
  apply RSysReachable.step a (next a) (next a) (.receipt a (effect r a s)) _ ()
  · exact RSysReachable.adversary a _ _ reach (.success en)
  · trivial
  · exact extracted_agrees a ()
  · rfl

/-- Construct all four operations and consume each receipt, starting without
any assumption that the final name exists. -/
theorem full_reachable (r : Recipe) (s : FS) (fresh : Enabled r .Stage s) :
    Reach r s (.Complete, .waiting .Complete (published r s)) := by
  have start : Reach r s (.Stage, .waiting .Stage s) := .init _ rfl
  have tmp : (stage r s).visible.link r.tmp = some r.inode := by
    simp [stage, FS.bind, FS.write, FS.chmod, FS.modify, Image.bind, Image.write, Image.chmod]
  have staged := advance r s .Stage s start fresh
  have synced := advance r s .SyncFile (stage r s) staged tmp
  have moved := advance r s .PublishName (sealed r s) synced tmp
  exact advance r s .SyncNamespace (renamed r s) moved (visible_rename_dst _ _ _ _)

theorem checked_publication_record (r : Recipe) (s : FS) (fresh : Enabled r .Stage s)
    (i : Image) (h : CrashImage (published r s) i) : i.link r.dst = some r.inode ∧ i.file r.inode = r.value :=
  reachable_record r s _ (full_reachable r s fresh) i h

/-- Even a crash between syscall completion and receipt consumption cannot
release a success event from the interrupted session. -/
theorem stopped_never_acknowledges (r : Recipe) (i : Image) :
    ¬ (world r).deliver (.stopped i) () ∧ ∀ v, ¬ (world r).adversary (.stopped i) v := by
  exact ⟨id, fun _ h => nomatch h⟩

/-! ## The stage normal form at the file-barrier cut

`RealStorage::write` can issue several positive writes, each partially
survivable before the file barrier. `create_replacing` only unlinks on
`AlreadyExists`. Do not equate those intermediate power images to `stage`.
At the successful fsync, however, every real image is an image of the
normal form, **including its entire namespace and all other inodes**. -/

def removeStale (r : Recipe) (s : FS) : FS :=
  if s.visible.link r.tmp = none then s else s.bind r.tmp none

/-- The exclusive create installs a fresh, empty inode with its effective
creation mode, which need not equal the mode later installed by fchmod. -/
def created (r : Recipe) (s : FS) (mode : Nat) : FS :=
  ((removeStale r s).initFile r.inode mode).bind r.tmp (some r.inode)

/-- Identical visible view and a forward simulation of all crash images.
`view` hides the one in-flight file *only until* the file barrier. -/
structure Simulates (view : Image → Image) (actual canonical : FS) : Prop where
  visible : view actual.visible = view canonical.visible
  cuts : ∀ i, CrashImage actual i → ∃ j, CrashImage canonical j ∧ view i = view j

private theorem Simulates.refl (view : Image → Image) (s : FS) : Simulates view s s :=
  ⟨rfl, fun i hi => ⟨i, hi, rfl⟩⟩

private theorem Simulates.trans {view a b c} (h : Simulates view a b) (g : Simulates view b c) :
    Simulates view a c := by
  refine ⟨h.visible.trans g.visible, fun i hi => ?_⟩
  obtain ⟨j, hj, e⟩ := h.cuts i hi
  obtain ⟨k, hk, e'⟩ := g.cuts j hj
  exact ⟨k, hk, e.trans e'⟩

private theorem Simulates.modify {view a b} (h : Simulates view a b) (f : Image → Image)
    (cong : ∀ i j, view i = view j → view (f i) = view (f j)) :
    Simulates view (a.modify f) (b.modify f) := by
  refine ⟨cong _ _ h.visible, fun i hi => ?_⟩
  rcases crash_modify_iff.mp hi with ⟨u, hu, e | e⟩ <;> subst i
  · obtain ⟨v, hv, eq⟩ := h.cuts u hu
    exact ⟨v, crash_modify_skip hv, eq⟩
  · obtain ⟨v, hv, eq⟩ := h.cuts u hu
    exact ⟨f v, crash_modify_take hv, cong _ _ eq⟩

private theorem erase_left (view f : Image → Image) (s : FS) (neutral : ∀ i, view (f i) = view i) :
    Simulates view (s.modify f) s :=
  ⟨neutral _, fun _ hi => crash_modify_frame view neutral hi⟩

private theorem erase_right (view f : Image → Image) (s : FS) (neutral : view (f s.visible) = view s.visible) :
    Simulates view s (s.modify f) :=
  ⟨neutral.symm, fun i hi => ⟨i, crash_modify_skip hi, rfl⟩⟩

private def mask (n : Nat) (i : Image) := i.initFile n 0

private theorem mask_write (i : Image) (n : Nat) (b : List UInt8) : mask n (i.write n b) = mask n i := by
  unfold mask Image.write Image.initFile
  congr 1
  funext k
  by_cases h : k = n <;> simp [h]

private theorem mask_chmod (i : Image) (n mode : Nat) : mask n (i.chmod n mode) = mask n i := by
  unfold mask Image.chmod Image.initFile
  congr 1
  funext k
  by_cases h : k = n <;> simp [h]

private theorem mask_init (i : Image) (n mode : Nat) : mask n (i.initFile n mode) = mask n i := by
  unfold mask Image.initFile
  congr 1
  funext k
  by_cases h : k = n <;> simp [h]

private theorem init_erases (s : FS) (n mode : Nat) : Simulates (mask n) (s.initFile n mode) s := by
  refine ⟨mask_init _ _ _, fun i hi => ?_⟩
  rcases hi with rfl | ⟨j, hj, rfl⟩
  · exact ⟨_, crash_visible _, mask_init _ _ _⟩
  · exact ⟨_, hj, mask_init _ _ _⟩

private theorem chunks_erase {n off b s t} (ch : Chunks n off b s t) : Simulates (mask n) t s := by
  induction ch with
  | nil => exact Simulates.refl _ _
  | @cons off s t chunk rest _ _ ih =>
    exact ih.trans (erase_left _ _ s (fun i => mask_write i n _))

private theorem creation_erases (r : Recipe) (s : FS) (mode : Nat) :
    Simulates (mask r.inode) (created r s mode) ((s.bind r.tmp none).bind r.tmp (some r.inode)) := by
  have stale : Simulates (mask r.inode) (removeStale r s) (s.bind r.tmp none) := by
    unfold removeStale
    split
    · rename_i missing
      apply erase_right
      have unchanged : s.visible.bind r.tmp none = s.visible := by
        unfold Image.bind
        congr 1
        funext l
        by_cases eq : l = r.tmp <;> simp [eq, missing]
      exact congrArg _ unchanged
    · exact Simulates.refl _ _
  exact ((init_erases _ r.inode mode).trans stale).modify (fun i => i.bind r.tmp (some r.inode))
    (fun _ _ eq => congrArg (fun i : Image => i.bind r.tmp (some r.inode)) eq)

private theorem unmask {n : Nat} {i j : Image} (eq : mask n i = mask n j) (file : i.file n = j.file n) :
    i = j := by
  have hl := congrArg Image.link eq
  change i.link = j.link at hl
  have hf : i.file = j.file := by
    funext k
    by_cases h : k = n
    · simpa [h] using file
    · simpa [mask, Image.initFile, h] using congrArg (fun v : Image => v.file k) eq
  cases i; cases j; congr

private theorem mask_copyFile (i v : Image) (n : Nat) : mask n (i.copyFile v n) = mask n i := by
  unfold mask Image.copyFile Image.initFile
  congr 1
  funext k
  by_cases h : k = n <;> simp [h]

/-- The pinned file descriptor starts at offset zero. Every complete host
write-all partition followed by fchmod and file sync refines our normal
form, for fresh **or stale** staging names and any effective creation mode.
No restriction on pre-existing bytes/mode or on the pre-existing final name. -/
theorem chunked_sync_refines (r : Recipe) (s : FS) (mode : Nat) {written : FS}
    (ch : Chunks r.inode 0 r.value.bytes (created r s mode) written) :
    Simulates id ((written.chmod r.inode r.value.mode).syncFile r.inode)
      ((stage r s).syncFile r.inode) := by
  let named := (s.bind r.tmp none).bind r.tmp (some r.inode)
  have addWrite := erase_right (mask r.inode) (fun i => i.write r.inode r.value.bytes) named (mask_write _ _ _)
  have addMode := erase_right (mask r.inode) (fun i => i.chmod r.inode r.value.mode)
    (named.write r.inode r.value.bytes) (mask_chmod _ _ _)
  have erased : Simulates (mask r.inode) (written.chmod r.inode r.value.mode) (stage r s) :=
    ((((erase_left _ _ written (fun i => mask_chmod i _ _)).trans (chunks_erase ch)).trans
      (creation_erases r s mode)).trans addWrite).trans addMode
  have empty : ((created r s mode).visible.file r.inode).bytes = [] := by
    simp [created, FS.bind, FS.modify, Image.bind, FS.initFile, Image.initFile]
  have bytes := ch.appended (by rw [empty]; rfl)
  rw [empty, List.nil_append] at bytes
  have file : ((written.chmod r.inode r.value.mode).visible.file r.inode) = (stage r s).visible.file r.inode := by
    simp [stage, FS.write, FS.modify, Image.write, FS.chmod, Image.chmod, bytes]
  refine ⟨unmask erased.visible file, fun i hi => ?_⟩
  obtain ⟨a, ha, rfl⟩ := crash_syncFile_iff.mp hi
  obtain ⟨b, hb, eq⟩ := erased.cuts a ha
  refine ⟨_, crash_syncFile_iff.mpr ⟨b, hb, rfl⟩, unmask (n := r.inode) ?_ ?_⟩
  · simpa only [id_eq, mask_copyFile] using eq
  · simpa [Image.copyFile] using file

/-! ## One finite, fresh run and two independent barrier-removal cuts -/

def freshImage : Image := ⟨fun _ => none, fun _ => ⟨[], 0⟩⟩
def exampleRecord : Recipe := ⟨⟨1, 2⟩, ⟨1, 3⟩, 9, ⟨[71], 384⟩⟩

theorem fresh_reachable : freshImage.link exampleRecord.dst = none ∧
    Reach exampleRecord (FS.stable freshImage)
      (.Complete, .waiting .Complete (published exampleRecord (FS.stable freshImage))) := by
  refine ⟨rfl, full_reachable _ _ ⟨rfl, by decide, ?_⟩⟩
  intro i hi l
  obtain rfl := crash_stable.mp hi
  simp [freshImage]

private theorem skip_stage (r : Recipe) {s i} (h : CrashImage s i) : CrashImage (stage r s) i :=
  crash_modify_skip (crash_modify_skip (crash_modify_skip (crash_modify_skip h)))

/-- Without the file barrier, a real directory barrier installs the final
name, but the inode reached through it has the original empty bytes. -/
theorem omitted_file_barrier_loses_bytes :
    let r := exampleRecord
    let staged := stage r (FS.stable freshImage)
    let named := staged.rename r.tmp r.dst r.inode
    ∃ i, CrashImage (named.syncDir r.dst.parent) i ∧ i.link r.dst = some r.inode ∧
      (i.file r.inode).bytes = [] ∧ (i.file r.inode).bytes ≠ r.value.bytes := by
  dsimp only
  let r := exampleRecord
  let staged := stage r (FS.stable freshImage)
  let named := staged.rename r.tmp r.dst r.inode
  have old : CrashImage staged freshImage := skip_stage r (crash_visible (FS.stable freshImage))
  have hn : CrashImage named freshImage :=
    (withinDir_rename_atomic (s := staged) (src := r.tmp) (dst := r.dst) (n := r.inode) rfl).mpr
      ⟨_, old, Or.inl rfl⟩
  exact ⟨freshImage.copyDir named.visible r.dst.parent, crash_syncDir_iff.mpr ⟨_, hn, rfl⟩,
    by decide, rfl, by decide⟩

/-- Without the directory barrier the synced file is correct but unreachable:
the previously absent final name can remain absent after atomic rename. -/
theorem omitted_directory_barrier_loses_name :
    let r := exampleRecord
    let synced := (stage r (FS.stable freshImage)).syncFile r.inode
    ∃ i, CrashImage (synced.rename r.tmp r.dst r.inode) i ∧ i.link r.dst = none ∧ i.file r.inode = r.value := by
  dsimp only
  let r := exampleRecord
  let s := FS.stable freshImage
  let i := freshImage.copyFile (stage r s).visible r.inode
  have old : CrashImage (stage r s) freshImage := skip_stage r (crash_visible s)
  let synced := (stage r s).syncFile r.inode
  have hf : CrashImage synced i := crash_syncFile_iff.mpr ⟨_, old, rfl⟩
  exact ⟨i, (withinDir_rename_atomic (s := synced) (src := r.tmp) (dst := r.dst)
    (n := r.inode) rfl).mpr ⟨_, hf, Or.inl rfl⟩, rfl, by decide⟩

end Gripsack.Protocol.Publication
