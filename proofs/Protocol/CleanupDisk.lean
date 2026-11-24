import Storage

/-! The adapter's cleanup suffix, below the read/codec and leaf settlement boundary.
Locations are relative to a pinned journal inode; `records` is the complete, ordered
reader enumeration (including separately premised directory records). An ordinary
unlink has no implicit persistence guarantee. Both journal barriers are real calls. -/
namespace Gripsack.Protocol.CleanupDisk

open Gripsack.Storage

structure Layout where
  marker : Loc
  records : List Loc
  inside : ∀ l ∈ records, l.parent = marker.parent
  apart : marker ∉ records

inductive Op where
  | record (l : Loc)
  | sync
  | marker

def Layout.plan (j : Layout) : List Op :=
  j.records.map Op.record ++ [.sync, .marker, .sync]

def Op.command (j : Layout) : Op → Command
  | .record l => .unlink l
  | .sync => .syncDir j.marker.parent
  | .marker => .unlink j.marker

def Op.effect (j : Layout) : Op → FS → FS
  | .record l, s => s.bind l none
  | .sync, s => s.syncDir j.marker.parent
  | .marker, s => s.bind j.marker none

/-- Only `ENOENT` at the actual pinned marker name is normalized by the host.
In particular neither an arbitrary `Step.errorBefore` nor a failed record unlink
can deliver a successful reply. A `lostReply` is killed, not a reported I/O error. -/
inductive Call (j : Layout) (m : Machine) : Op → Status → Machine → Prop where
  | io {op r t} : Step m (op.command j) r t → Call j m op r t
  | enoent {p} : PinnedDir m p j.marker.parent → m.fs.visible.link j.marker = none →
      Call j m .marker .ok m

private theorem completed_effect {j op m t} (h : Completed m (Op.command j op) t) :
    t.fs = op.effect j m.fs := by
  cases op <;> cases h <;> rfl

private theorem Call.shape {j m op r t} (h : Call j m op r t) :
    (t.fs = m.fs ∧ (r ≠ .ok ∨ (op = .marker ∧ m.fs.visible.link j.marker = none))) ∨
    t.fs = op.effect j m.fs := by
  cases h with
  | enoent _ absent => exact Or.inl ⟨rfl, Or.inr ⟨rfl, absent⟩⟩
  | io step =>
    cases step with
    | completed h | errorAfter h => exact Or.inr (completed_effect h)
    | lostReply h => exact Or.inr (by simpa only [Machine.kill] using completed_effect h)
    | errorBefore | killedBefore => exact Or.inl ⟨rfl, Or.inl (by decide)⟩
    | writeAll bad _ _ _ _ => cases op <;> cases bad

inductive Walk (j : Layout) : Machine → List Op → Machine → Prop where
  | nil {m} : Walk j m [] m
  | cons {m op u ops t} : Call j m op .ok u → Walk j u ops t → Walk j m (op :: ops) t

/-- All cuts of this invocation. `stop` also admits a crash between calls;
`interrupted` has no continuation and includes death after the effect, before reply. -/
inductive Cut (j : Layout) : Machine → List Op → Machine → Prop where
  | stop {m ops} : Cut j m ops m
  | interrupted {m op r t ops} : Call j m op r t → r ≠ .ok → Cut j m (op :: ops) t
  | next {m op u ops t} : Call j m op .ok u → Cut j u ops t → Cut j m (op :: ops) t

theorem Walk.cut {j m ops t} (h : Walk j m ops t) : Cut j m ops t := by
  induction h with
  | nil => exact .stop
  | cons one _ ih => exact .next one ih

theorem Walk.thenCut {j m xs u ys t} (pre : Walk j m xs u) (post : Cut j u ys t) :
    Cut j m (xs ++ ys) t := by
  induction pre with
  | nil => exact post
  | cons one _ ih => exact .next one (ih post)

theorem Walk.split {j m xs ys t} (h : Walk j m (xs ++ ys) t) :
    ∃ u, Walk j m xs u ∧ Walk j u ys t := by
  induction xs generalizing m with
  | nil => exact ⟨_, .nil, h⟩
  | cons x xs ih =>
    cases h with | cons one more =>
      obtain ⟨u, pre, post⟩ := ih more
      exact ⟨u, .cons one pre, post⟩

theorem Cut.split {j m xs ys t} (h : Cut j m (xs ++ ys) t) :
    Cut j m xs t ∨ ∃ u, Walk j m xs u ∧ Cut j u ys t := by
  induction xs generalizing m with
  | nil => exact Or.inr ⟨_, .nil, h⟩
  | cons x xs ih =>
    cases h with
    | stop => exact Or.inl .stop
    | interrupted one bad => exact Or.inl (.interrupted one bad)
    | next one more =>
      rcases ih more with pre | ⟨u, pre, post⟩
      · exact Or.inl (.next one pre)
      · exact Or.inr ⟨u, .cons one pre, post⟩

def Op.Away (j : Layout) (l : Loc) : Op → Prop
  | .record k => l ≠ k
  | .sync => True
  | .marker => l ≠ j.marker

private theorem Call.link {j m op r t l v} (h : Call j m op r t) (away : op.Away j l)
    (stable : StableLink m.fs l v) : StableLink t.fs l v := by
  rcases h.shape with ⟨e, _⟩ | e <;> rw [e]
  · exact stable
  · cases op with
    | record | marker => exact stable.bind away
    | sync => exact stable.syncDir

private theorem Call.file {j m op r t n v} (h : Call j m op r t)
    (stable : StableFile m.fs n v) : StableFile t.fs n v := by
  rcases h.shape with ⟨e, _⟩ | e <;> rw [e]
  · exact stable
  · cases op with
    | record | marker => exact stable.bind
    | sync => exact stable.syncDir

theorem Cut.link {j m ops t l v} (h : Cut j m ops t) (away : ∀ op ∈ ops, op.Away j l)
    (stable : StableLink m.fs l v) : StableLink t.fs l v := by
  induction h with
  | stop => exact stable
  | interrupted one _ => exact one.link (away _ (by simp)) stable
  | next one _ ih => exact ih (fun _ hl => away _ (List.mem_cons_of_mem _ hl)) (one.link (away _ (by simp)) stable)

private theorem Call.absent {j m op r t l} (h : Call j m op r t) (stable : StableLink m.fs l none) :
    StableLink t.fs l none := by
  rcases h.shape with ⟨e, _⟩ | e <;> rw [e]
  · exact stable
  · cases op with
    | sync => exact stable.syncDir
    | record | marker =>
      apply All.modify stable
      intro i hi
      simp [Image.bind, hi]

theorem Cut.absent {j m ops t l} (h : Cut j m ops t) (stable : StableLink m.fs l none) :
    StableLink t.fs l none := by
  induction h with
  | stop => exact stable
  | interrupted one _ => exact one.absent stable
  | next one _ ih => exact ih (one.absent stable)

theorem Cut.frameFile {j m ops t n v} (h : Cut j m ops t) (stable : StableFile m.fs n v) :
    StableFile t.fs n v := by
  induction h with
  | stop => exact stable
  | interrupted one _ => exact one.file stable
  | next one _ ih => exact ih (one.file stable)

theorem Cut.frameLink {j m t l v} (h : Cut j m j.plan t) (marker : l ≠ j.marker)
    (record : l ∉ j.records) (stable : StableLink m.fs l v) : StableLink t.fs l v := by
  apply h.link _ stable
  intro op hop
  simp only [Layout.plan, List.mem_append, List.mem_map, List.mem_cons, List.not_mem_nil, or_false] at hop
  rcases hop with ⟨k, hk, rfl⟩ | rfl | rfl | rfl
  · exact fun e => record (e ▸ hk)
  · trivial
  · exact marker
  · trivial

private theorem Call.keepsMissing {j m op r t l} (h : Call j m op r t) (gone : m.fs.visible.link l = none) :
    t.fs.visible.link l = none := by
  rcases h.shape with ⟨e, _⟩ | e <;> rw [e]
  · exact gone
  · cases op with
    | sync => exact gone
    | record k => by_cases e : l = k <;> simp [Op.effect, FS.bind, FS.modify, Image.bind, e, gone]
    | marker => by_cases e : l = j.marker <;> simp [Op.effect, FS.bind, FS.modify, Image.bind, e, gone]

private theorem Walk.keepsMissing {j m ops t l} (h : Walk j m ops t) (gone : m.fs.visible.link l = none) :
    t.fs.visible.link l = none := by
  induction h with
  | nil => exact gone
  | cons one _ ih => exact ih (one.keepsMissing gone)

private theorem Walk.recordsMissing {j m t} {ls : List Loc} (h : Walk j m (ls.map Op.record) t) :
    ∀ l ∈ ls, t.fs.visible.link l = none := by
  induction ls generalizing m with
  | nil => simp
  | cons k ks ih =>
    cases h with | cons one more =>
      intro l hl
      rcases List.mem_cons.mp hl with rfl | hl
      · apply more.keepsMissing
        rcases one.shape with ⟨_, bad | ⟨bad, _⟩⟩ | e
        · exact False.elim (bad rfl)
        · cases bad
        · simp [e, Op.effect, FS.bind, FS.modify, Image.bind]
      · exact ih more l hl

private theorem Call.syncOk {j m t} (h : Call j m .sync .ok t) : t.fs = m.fs.syncDir j.marker.parent := by
  rcases h.shape with ⟨_, bad | ⟨bad, _⟩⟩ | e
  · exact False.elim (bad rfl)
  · cases bad
  · exact e

private theorem Call.markerMissing {j m t} (h : Call j m .marker .ok t) : t.fs.visible.link j.marker = none := by
  rcases h.shape with ⟨e, bad | ⟨_, gone⟩⟩ | e
  · exact False.elim (bad rfl)
  · exact e ▸ gone
  · simp [e, Op.effect, FS.bind, FS.modify, Image.bind]

private theorem syncRecords {j m u t} (pre : Walk j m (j.records.map Op.record) u)
    (one : Call j u .sync .ok t) : ∀ l ∈ j.records, StableLink t.fs l none := by
  intro l hl i hi
  rw [one.syncOk] at hi
  exact (crash_syncDir_link hi (j.inside l hl)).trans (pre.recordsMissing l hl)

/-- The pre/post-record-barrier split holds at machine level, not only per
image. `original` may include additional names already visibly missing when
this process rescanned after a kill. They, too, are flushed by barrier #1. -/
theorem Cut.markedOrGone {j m t n} {original : List Loc} (h : Cut j m j.plan t)
    (marked : StableLink m.fs j.marker (some n))
    (inside : ∀ l ∈ original, l.parent = j.marker.parent)
    (scanned : ∀ l ∈ original, l ∈ j.records ∨ m.fs.visible.link l = none) :
    StableLink t.fs j.marker (some n) ∨ ∀ l ∈ original, StableLink t.fs l none := by
  have separated : ∀ op ∈ j.records.map Op.record, op.Away j j.marker := by
    rintro _ hop
    obtain ⟨l, hl, rfl⟩ := List.mem_map.mp hop
    exact fun e => j.apart (e ▸ hl)
  rcases Cut.split h with pre | ⟨u, pre, post⟩
  · exact Or.inl (pre.link separated marked)
  · have keep := pre.cut.link separated marked
    cases post with
    | stop => exact Or.inl keep
    | interrupted one _ => exact Or.inl (one.link trivial keep)
    | next one rest =>
      right
      intro l hl
      apply rest.absent
      intro i hi
      rw [one.syncOk] at hi
      rw [crash_syncDir_link hi (inside l hl)]
      rcases scanned l hl with found | gone
      · exact pre.recordsMissing l found
      · exact pre.keepsMissing gone

/-- On a marked committed run, the no-marker legacy branch cannot see even one
stale record. This includes every admitted image after an error or lost reply. -/
theorem Cut.marked {j m t n} (h : Cut j m j.plan t) (marked : StableLink m.fs j.marker (some n)) :
    All t.fs (fun i => i.link j.marker = some n ∨ ∀ l ∈ j.records, i.link l = none) := by
  rcases h.markedOrGone marked j.inside (fun _ hl => Or.inl hl) with keep | gone
  · exact fun i hi => Or.inl (keep i hi)
  · exact fun i hi => Or.inr (fun l hl => gone l hl i hi)

/-- Both acknowledged barriers: no marker *and* no record across all power
images, regardless of the initial marker (including a permitted legacy journal). -/
theorem Walk.clean {j m t} (h : Walk j m j.plan t) :
    StableLink t.fs j.marker none ∧ ∀ l ∈ j.records, StableLink t.fs l none := by
  obtain ⟨u, pre, post⟩ := Walk.split h
  cases post with | cons sync1 more =>
    cases more with | cons unlink more =>
      cases more with | cons sync2 done =>
        cases done
        constructor
        · intro i hi
          rw [sync2.syncOk] at hi
          exact (crash_syncDir_link hi rfl).trans unlink.markerMissing
        · intro l hl
          exact sync2.link trivial (unlink.link (fun e => j.apart (e ▸ hl)) (syncRecords pre sync1 l hl))

private theorem completed_kind {j m op t} (h : Completed m (Op.command j op) t) : t.kind = m.kind := by
  cases op <;> cases h <;> rfl

theorem Call.kind {j m op r t} (h : Call j m op r t) : t.kind = m.kind := by
  cases h with
  | enoent => rfl
  | io step =>
    cases step with
    | completed h | errorAfter h => exact completed_kind h
    | lostReply h => simpa only [Machine.kill] using completed_kind h
    | errorBefore | killedBefore => rfl
    | writeAll bad _ _ _ _ => cases op <;> cases bad

theorem Cut.kind {j m ops t} (h : Cut j m ops t) : t.kind = m.kind := by
  induction h with
  | stop => rfl
  | interrupted one _ => exact one.kind
  | next one _ ih => exact ih.trans one.kind

/-- Restart opens a fresh session, not a new durability choice at kill. The
same statement transports settlement, no-misclassification, or completed cleanup. -/
theorem restart_all {m t k P} (crash : Crash m k t) (safe : All m.fs P) : All t.fs P := by
  cases crash with
  | kill => exact safe
  | power h => intro i hi; exact (crash_stable.mp hi) ▸ safe _ h

/-- One interrupted cleanup, a kill or any power image, read-only repinning /
rescanning, and a possibly interrupted second cleanup. Omitted original names
need only be *visibly* absent on rescan, not already durably absent on kill. -/
theorem restart_cut_marked {j j' m t r m' t' n k}
    (first : Cut j m j.plan t) (marked : StableLink m.fs j.marker (some n))
    (crash : Crash t k r) (reopen : m'.fs = r.fs) (same : j'.marker = j.marker)
    (scanned : ∀ l ∈ j.records, l ∈ j'.records ∨ m'.fs.visible.link l = none)
    (second : Cut j' m' j'.plan t') :
    All t'.fs (fun i => i.link j.marker = some n ∨ ∀ l ∈ j.records, i.link l = none) := by
  rcases first.markedOrGone marked j.inside (fun _ hl => Or.inl hl) with keep | gone
  · have keep' : StableLink m'.fs j'.marker (some n) := same ▸ reopen ▸ restart_all crash keep
    rcases second.markedOrGone keep' (fun l hl => same ▸ j.inside l hl) scanned with keep'' | gone'
    · exact fun i hi => Or.inl (same ▸ keep'' i hi)
    · exact fun i hi => Or.inr (fun l hl => gone' l hl i hi)
  · exact fun i hi => Or.inr (fun l hl => second.absent (reopen ▸ restart_all crash (gone l hl)) i hi)

/-! ## Reachability and historical controls

The bytes here follow `journal_cleanup.rs`'s small test wire, *not* a new store
codec. `dest.json` says restore ONE, before ONE, intended TWO. The selected
`current` symlink, the regular file TWO and both journal names start durable. -/
namespace Example

def record : Loc := ⟨0, 10⟩
def marker : Loc := ⟨0, 11⟩
def dest : Loc := ⟨1, 12⟩
def current : Loc := ⟨1, 13⟩
def manifest : Loc := ⟨7, 14⟩
def bytes (s : String) := s.toUTF8.data.toList
def one : FileData := ⟨bytes "one", 0o644⟩
def two : FileData := ⟨bytes "two", 0o644⟩
def undo : FileData := ⟨bytes "dest\nF:one\nF:one\nF:two", 0o600⟩
def run : FileData := ⟨bytes "generations/1\ngenerations/2", 0o600⟩
def selected : FileData := ⟨bytes "manifest-two", 0o644⟩
def layout : Layout := ⟨marker, [record], by simp [record, marker], by decide⟩

def initial : Image :=
  ⟨(fun l => if l = record then some 2 else if l = marker then some 3
    else if l = dest then some 4 else if l = current then some 5
    else if l = manifest then some 6 else if l = ⟨1, 15⟩ then some 0
    else if l = ⟨1, 16⟩ then some 8 else if l = ⟨8, 17⟩ then some 7 else none),
   (fun n => if n = 2 then undo else if n = 3 then run else if n = 4 then two
    else if n = 6 then selected else ⟨[], 0⟩)⟩

def start : Machine :=
  ⟨FS.stable initial, (fun n => if n = 0 ∨ n = 1 ∨ n = 7 ∨ n = 8 then some .directory
    else if n = 5 then some (.symlink (bytes "generations/2")) else some .regular),
    1, some ((Session.root 1).pin 0)⟩

private def dropName (m : Machine) (l : Loc) : Machine := {m with fs := m.fs.bind l none}
private def sync (m : Machine) : Machine := {m with fs := m.fs.syncDir 0}

private theorem unlinkAt {m : Machine} {l : Loc} {n : Nat}
    (session : m.session = start.session) (dir : m.kind 0 = some .directory) (parent : l.parent = 0)
    (found : m.fs.visible.link l = some n) (kind : m.kind n = some .regular) :
    Completed m (.unlink l) (dropName m l) :=
  .unlink ⟨session, Or.inl parent, parent ▸ dir⟩ found kind (by decide)

private theorem syncAt {m : Machine} (session : m.session = start.session) (dir : m.kind 0 = some .directory) :
    Completed m (.syncDir 0) (sync m) :=
  .syncDir ⟨session, Or.inl rfl, dir⟩

private def u₁ := dropName start record
private def u₂ := sync u₁
private def u₃ := dropName u₂ marker
private def u₄ := sync u₃

private theorem firstUnlink : Completed start (.unlink record) u₁ :=
  unlinkAt (n := 2) rfl rfl rfl rfl rfl

private theorem modernWalk : Walk layout start layout.plan u₄ :=
  .cons (.io (.completed firstUnlink))
    (.cons (.io (.completed (syncAt rfl rfl)))
    (.cons (.io (.completed (unlinkAt (n := 3) rfl rfl rfl rfl rfl)))
    (.cons (.io (.completed (syncAt rfl rfl))) .nil)))

/-- A nonempty marked journal really executes all four checked operations. -/
theorem reachable_complete : ∃ t, Walk layout start layout.plan t ∧
    StableLink t.fs marker none ∧ StableLink t.fs record none := by
  exact ⟨_, modernWalk, modernWalk.clean.1, modernWalk.clean.2 record (by simp [layout])⟩

/-- The second barrier is needed for a durable *marker* removal. The record
cannot come back here (barrier #1), but the marker unlink can still be lost. -/
theorem before_second_barrier_marker_can_return : ∃ t i,
    Walk layout start [.record record, .sync, .marker] t ∧ CrashImage t.fs i ∧
    t.fs.visible.link marker = none ∧ i.link marker = some 3 ∧ i.link record = none := by
  exact ⟨u₃, u₂.fs.visible, .cons (.io (.completed firstUnlink))
    (.cons (.io (.completed (syncAt rfl rfl)))
      (.cons (.io (.completed (unlinkAt (n := 3) rfl rfl rfl rfl rfl))) .nil)),
    crash_modify_skip (crash_visible u₂.fs), rfl, rfl, rfl⟩

def historicalPlan (markerFirst : Bool) : List Op :=
  if markerFirst then [.marker, .record record] else [.record record, .marker]

/-- The target selection (symlink and manifest) and TWO are intact. It is the
surviving undo record with no marker, not a torn destination, that is dangerous. -/
def Resurrected (m : Machine) : Prop :=
  m.fs.visible.link marker = none ∧ m.fs.visible.read record = some undo ∧
  m.fs.visible.read dest = some two ∧ m.fs.visible.link current = some 5 ∧
  m.kind 5 = some (.symlink (bytes "generations/2")) ∧ m.fs.visible.read manifest = some selected

private theorem bothUnlinked (markerFirst : Bool) : ∃ m, Walk layout start (historicalPlan markerFirst) m ∧
    m.fs.visible.link marker = none ∧ m.fs.visible.link record = none ∧
    ∃ reboot, Crash m .power reboot ∧ Resurrected reboot := by
  let bad := initial.bind marker none
  cases markerFirst with
  | false =>
    let m := dropName u₁ marker
    have hi : CrashImage m.fs bad := crash_modify_take (s := u₁.fs) (f := fun i => i.bind marker none)
      (crash_modify_skip (crash_visible start.fs))
    exact ⟨m, .cons (.io (.completed firstUnlink))
      (.cons (.io (.completed (unlinkAt (n := 3) rfl rfl rfl rfl rfl))) .nil),
      rfl, rfl, m.power bad, .power hi, rfl, rfl, rfl, rfl, rfl, rfl⟩
  | true =>
    let u := dropName start marker
    let m := dropName u record
    have hi : CrashImage m.fs bad := crash_modify_skip (s := u.fs) (f := fun i => i.bind record none)
      (crash_modify_take (s := start.fs) (f := fun i => i.bind marker none) (crash_visible _))
    exact ⟨m, .cons (.io (.completed (unlinkAt (n := 3) rfl rfl rfl rfl rfl)))
      (.cons (.io (.completed (unlinkAt (n := 2) rfl rfl rfl rfl rfl))) .nil),
      rfl, rfl, m.power bad, .power hi, rfl, rfl, rfl, rfl, rfl, rfl⟩

/-- 0.19.1: both unlinks returned success and `commit_run` then returned;
power may retain the marker unlink independently. Kill sees both names gone. -/
theorem historical_0191_after_return (markerFirst : Bool) :
    ∃ m reboot, Walk layout start (historicalPlan markerFirst) m ∧
      Crash m .kill m.kill ∧ m.kill.fs.visible.link marker = none ∧
      m.kill.fs.visible.link record = none ∧ Crash m .power reboot ∧ Resurrected reboot := by
  obtain ⟨m, h, hm, hr, reboot, hp, bad⟩ := bothUnlinked markerFirst
  exact ⟨m, reboot, h, .kill, hm, hr, hp, bad⟩

/-- 0.19.2: the iteration finished but the one *trailing* barrier has not.
Error-before on that barrier gives the very same cut; not an acknowledged sync. -/
theorem historical_0192_before_trailing_sync (markerFirst : Bool) :
    ∃ m reboot, Walk layout start (historicalPlan markerFirst) m ∧
      Step m (.syncDir 0) .error m ∧ Cut layout start (historicalPlan markerFirst ++ [.sync]) m ∧
      Crash m .power reboot ∧ Resurrected reboot := by
  obtain ⟨m, h, _, _, reboot, hp, bad⟩ := bothUnlinked markerFirst
  exact ⟨m, reboot, h, .errorBefore, h.thenCut (.interrupted (.io .errorBefore) (by decide)), hp, bad⟩

/-- A genuinely markerless, nonempty legacy journal after settlement. Even if
its just-unlinked entry reappears on reboot, the destination *already is* the
entry's prior (same name, bytes and mode). A successful retry may accept ENOENT
for the marker; success does not require manufacturing a linked marker. -/
theorem legacy_surviving_record_at_prior :
    ∃ m mid reboot done,
      m.fs.visible.link marker = none ∧ m.fs.visible.read record = some undo ∧
      Walk layout m [.record record] mid ∧ Crash mid .power reboot ∧
      reboot.fs.visible.link marker = none ∧ reboot.fs.visible.read record = some undo ∧
      reboot.fs.visible.read dest = some one ∧
      Walk layout m layout.plan done ∧ StableLink done.fs record none ∧ StableLink done.fs marker none := by
  let img := (initial.bind marker none).write 4 one.bytes
  let m : Machine := { start with fs := FS.stable img }
  let mid := dropName m record
  have entry : Call layout m (.record record) .ok mid :=
    .io (.completed (unlinkAt (n := 2) rfl rfl rfl rfl rfl))
  have missing : Call layout (sync mid) .marker .ok (sync mid) :=
    .enoent ⟨rfl, Or.inl rfl, rfl⟩ rfl
  have finish : Walk layout m layout.plan (sync (sync mid)) :=
    .cons entry (.cons (.io (.completed (syncAt rfl rfl)))
      (.cons missing (.cons (.io (.completed (syncAt rfl rfl))) .nil)))
  exact ⟨m, mid, mid.power img, _, rfl, rfl, .cons entry .nil,
    .power (crash_modify_skip (crash_visible m.fs)), rfl, rfl, rfl, finish,
    finish.clean.2 record (by simp [layout]), finish.clean.1⟩

end Example

end Gripsack.Protocol.CleanupDisk
