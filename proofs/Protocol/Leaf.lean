import Protocol.Capture
import Kernel.JournalRun

/-!
# Write-ahead evidence at admission of a leaf mutation (storage P1)

`Run` and `run_step` below are the existing execution relation and the
Aeneas translation, not a second transaction machine. `Walk` interprets its
issued commands, in order, up to the cut *before* executing `Mutate`.
`Host` models successful calls while the writer is live, not an oracle that
acknowledges durability. A failed call returns `Failed` (terminal, proved
below); process death delivers no further event from that session.

The wire codec, stable home capability, path/name/digest interning and
binding the caller's `Leaf` to its `UnitWire` are ring-2 premises. Wire v2
has **no transaction ID or ordinal in the entry**. The marker supplies the
transaction, the entry names the destination, and the caller supplies the
path for this ordinal (the store adapter uses a local DestId of zero).
`Original` models `Entry::new/advance`: even a second change to the same
path must retain the first capture, not just the immediate before-image.
-/
namespace Gripsack.Protocol.Leaf

open Gripsack.Storage Gripsack.Protocol.Publication Gripsack.Kernel.JournalRun
open gripsack_kernel Aeneas Aeneas.Std Result

inductive PriorRef where
  | absent
  | link (target : String)
  | file (hash : ids.Digest) (mode : Nat)

structure Entry where
  path : String
  original : PriorRef
  before : PriorRef
  intended : recover.Intended

structure Context where
  journal : Directory
  priorDir : Loc
  marker : Recipe
  inside : marker.dst.parent = journal.inode
  previous : Option selection.SelectionIdentity
  target : selection.SelectionIdentity
  path : U32 → String
  facts : U32 → journal_run.UnitFacts
  hash : List UInt8 → ids.Digest
  fileId : List UInt8 → Nat → ids.Digest
  linkId : String → ids.Digest
  blobName : ids.Digest → Nat
  entryName : String → Nat
  decodeMarker : List UInt8 → Option (Option selection.SelectionIdentity × selection.SelectionIdentity)
  decodeEntry : List UInt8 → Option Entry
  markerWire : decodeMarker marker.value.bytes = some (previous, target)

def Prior.ref (k : Context) : Prior → PriorRef
  | .absent => .absent
  | .link t => .link t
  | .file _ b m => .file (k.hash b.value.bytes) m

def Prior.live (k : Context) : Prior → recover.Live
  | .absent => .Absent
  | .link t => .Obj (.Link (k.linkId t))
  | .file _ b m => .Obj (.File (k.fileId b.value.bytes m))

structure Packet where
  unit : U32
  path : String
  original : Prior
  before : Prior
  record : Recipe

def Prior.Located (k : Context) (p : Prior) : Prop :=
  ∀ d r m, p = .file d r m → d.loc = k.priorDir ∧
    r.dst = ⟨d.inode, k.blobName (k.hash r.value.bytes)⟩ ∧ r.value.mode = 0o600 ∧ d.loc ≠ r.tmp ∧ d.loc ≠ r.dst

/-- Successful encoding, observation and physical name-to-digest mapping.
These are facts about bytes *before* they are staged, not persistence laws. -/
structure Matches (k : Context) (q : Packet) : Prop where
  path : q.path = k.path q.unit
  location : q.record.dst = ⟨k.journal.inode, k.entryName q.path⟩
  wire : k.decodeEntry q.record.value.bytes = some ⟨q.path, q.original.ref k, q.before.ref k, (k.facts q.unit).intended⟩
  blob : q.before.Located k

/-- The latest prior entry for this exact path; `history` is newest first. -/
inductive Original (path : String) (before : Prior) : List Packet → Prior → Prop where
  | first : Original path before [] before
  | same {q qs} : q.path = path → Original path before (q :: qs) q.original
  | next {q qs p} : q.path ≠ path → Original path before qs p → Original path before (q :: qs) p

theorem Original.source {d b h p} (ho : Original d b h p) : p = b ∨ ∃ q ∈ h, p = q.original := by
  induction ho with
  | first => exact Or.inl rfl
  | same => exact Or.inr ⟨_, by simp, rfl⟩
  | next _ _ ih =>
    rcases ih with e | ⟨q, hq, e⟩
    · exact Or.inl e
    · exact Or.inr ⟨q, List.mem_cons_of_mem _ hq, e⟩

structure World where
  disk : FS
  beginCall : Bool
  history : List Packet

def selected (k : Context) (w : World) : journal_run.Record → Option Recipe
  | .Marker => if w.beginCall then some k.marker else none
  | .Entry => w.history.head?.map Packet.record

/-- Metadata cells private to the writer (S4), including previously captured
blobs. Protect names and file inodes separately: disjoint names alone would
miss a hard-link alias. This is a footprint, not a durability premise. -/
def Protected (k : Context) (w : World) (f : Fact) : Prop :=
  f = k.journal.fact ∨ (w.beginCall = false ∧ f ∈ recordFacts k.marker) ∨
  ∃ q ∈ w.history, f ∈ q.original.facts ++ q.before.facts

structure Receipt where
  ops : List Op
  packet : Option Packet := none
  leavesBegin : Bool := false

def Receipt.apply (r : Receipt) (w : World) : World :=
  ⟨execOps w.disk r.ops, w.beginCall && !r.leavesBegin, r.packet.toList ++ w.history⟩

def Receipt.renewed (r : Receipt) : List Fact := r.packet.toList.flatMap (fun q => q.before.facts)

/-- Effects of a *successful* call. `ready` yields back to the caller;
`capture` checks the observation again. `landed` permits arbitrary destination
operations under the private-state footprint, including an external S5 race.
Cleanup has no transition here: P1 ends before authorized retirement. -/
inductive Host (k : Context) (w : World) : Ev → Cmd → Ev → Receipt → Prop where
  | publish {e rec a r} : selected k w rec = some r → Enabled r a w.disk →
      Host k w e (.Publish rec a) .Done ⟨[.publish r a], none, false⟩
  | ready (e) (f : journal_run.UnitFacts) : Host k w e .Ready (.Unit f) ⟨[], none, true⟩
  | observe (u : U32) (x : recover.Live) : Host k w (.Unit (k.facts u))
      (.Observe u (k.facts u).dest) (.Observed (k.facts u).dest x) ⟨[], none, false⟩
  | capture (e) (q : Packet) (ops : List Op) : Matches k q → Original q.path q.before w.history q.original →
      Capture q.before w.disk ops → Host k w e (.CapturePrior q.unit (k.facts q.unit).dest (q.before.live k))
        .Done ⟨ops, some q, false⟩
  | landed (e u d x i y ops) : Host k w e (.Mutate u d x i) (.Landed y) ⟨ops, none, false⟩

/-- Checked separation of the actual calls: they may only touch old private
cells when this capture itself re-seals that cell (deduplicated blobs). -/
inductive Answer (k : Context) (w : World) (e : Ev) (c : Cmd) (next : Ev) : World → Prop where
  | done {r} : Host k w e c next r →
      (∀ f, Protected k w f → f ∈ r.renewed ∨ ∀ op ∈ r.ops, op.Away f) → Answer k w e c next (r.apply w)

inductive Walk (k : Context) : World → Ev → Log → World → Ev → Prop where
  | nil (w e) : Walk k w e [] w e
  | cons {w e c n v log t last} : Answer k w e c n v → Walk k v n log t last →
      Walk k w e ((e, c) :: log) t last

theorem Walk.split {k w e l₁ l₂ t n} (h : Walk k w e (l₁ ++ l₂) t n) :
    ∃ v ev, Walk k w e l₁ v ev ∧ Walk k v ev l₂ t n := by
  induction l₁ generalizing w e with
  | nil => exact ⟨_, _, .nil _ _, h⟩
  | cons x xs ih =>
    cases h with | cons a rest =>
      obtain ⟨v, ev, pre, post⟩ := ih rest
      exact ⟨v, ev, .cons a pre, post⟩

theorem Receipt.protects {k w f} (r : Receipt) (h : Protected k w f) : Protected k (r.apply w) f := by
  rcases h with h | ⟨hb, h⟩ | ⟨q, hq, h⟩
  · exact Or.inl h
  · exact Or.inr (Or.inl ⟨by simp [Receipt.apply, hb], h⟩)
  · exact Or.inr (Or.inr ⟨q, List.mem_append_right _ hq, h⟩)

theorem Host.renew {k w e c n r} (h : Host k w e c n r) : ∀ f ∈ r.renewed, Fact.Stable f (r.apply w).disk := by
  cases h with
  | capture _ q ops m _ hc =>
    exact fun f hf => capture_durable hc (fun d b mode eq => (m.blob d b mode eq).2.2.2) f
      (by simpa [Receipt.renewed] using hf)
  | _ => simp [Receipt.renewed]

theorem Answer.frame {k w e c n t f} (a : Answer k w e c n t) (hp : Protected k w f)
    (hs : f.Stable w.disk) : Protected k t f ∧ f.Stable t.disk := by
  cases a with | done act quiet =>
    refine ⟨Receipt.protects _ hp, ?_⟩
    rcases quiet f hp with hf | away
    · exact act.renew f hf
    · exact exec_frame hs away

theorem Walk.frame {k w e log t n f} (h : Walk k w e log t n) (hp : Protected k w f)
    (hs : f.Stable w.disk) : f.Stable t.disk := by
  induction h with
  | nil => exact hs
  | cons a _ ih => exact ih (a.frame hp hs).1 (a.frame hp hs).2

def Saved (w : World) : Prop := ∀ q ∈ w.history, ∀ f ∈ q.original.facts ++ q.before.facts, f.Stable w.disk

theorem Answer.saved {k w e c n t} (a : Answer k w e c n t) (h : Saved w) : Saved t := by
  have old : ∀ q ∈ w.history, ∀ f ∈ q.original.facts ++ q.before.facts, Fact.Stable f t.disk :=
    fun q hq f hf => (a.frame (Or.inr (Or.inr ⟨q, hq, hf⟩)) (h q hq f hf)).2
  cases a with | done act _ =>
    have renewed := act.renew
    cases act with
    | capture e q ops m orig hc =>
      intro z hz f hf
      change z ∈ q :: w.history at hz
      rcases List.mem_cons.mp hz with rfl | hz
      · have hb : ∀ f ∈ z.before.facts, Fact.Stable f (execOps w.disk ops) := fun f h =>
          renewed f (by simpa [Receipt.renewed] using h)
        rcases List.mem_append.mp hf with h0 | h1
        · rcases orig.source with eq | ⟨p, hp, eq⟩
          · exact hb f (eq ▸ h0)
          · exact old p hp f (List.mem_append_left _ (eq ▸ h0))
        · exact hb f h1
      · exact old z hz f hf
    | _ => exact old

theorem Walk.saved {k w e log t n} (h : Walk k w e log t n) (hs : Saved w) : Saved t := by
  induction h with | nil => exact hs | cons a _ ih => exact ih (a.saved hs)

/-- The file referred to by an original prior is located by the same raw
digest even after `Entry::advance` replaced its immediate before-image. -/
theorem Walk.original_locations {k w e log t n} (h : Walk k w e log t n)
    (old : ∀ q ∈ w.history, q.original.Located k) : ∀ q ∈ t.history, q.original.Located k := by
  induction h with
  | nil => exact old
  | cons ans _ ih =>
    apply ih
    cases ans with | done act _ =>
      cases act with
      | capture _ q _ m orig _ =>
        intro z hz
        change z ∈ q :: _ at hz
        rcases List.mem_cons.mp hz with rfl | hz
        · rcases orig.source with eq | ⟨p, hp, eq⟩
          · exact eq ▸ m.blob
          · exact eq ▸ old p hp
        · exact old _ hz
      | _ => exact old

def Prior.Recoverable (k : Context) (img : Image) : Prior → Prop
  | .file dir r _ =>
      img.link k.priorDir = some dir.inode ∧
      img.read ⟨dir.inode, k.blobName (k.hash r.value.bytes)⟩ = some ⟨r.value.bytes, 0o600⟩
  | .absent | .link _ => True  -- fully represented in the decoded record

theorem Prior.recoverable {k img p} (loc : Prior.Located k p)
    (has : ∀ f ∈ p.facts, Fact.Holds f img) : p.Recoverable k img := by
  cases p with
  | absent | link => trivial
  | file dir r m =>
    obtain ⟨root, binding, mode, _⟩ := loc dir r m rfl
    have parent : img.link dir.loc = some dir.inode := has dir.fact (by simp [Prior.facts])
    have named : img.link r.dst = some r.inode := has (.binding r.dst r.inode) (by simp [Prior.facts, recordFacts])
    have data : img.file r.inode = r.value := has (.content r.inode r.value) (by simp [Prior.facts, recordFacts])
    exact ⟨root ▸ parent, by rw [← binding]; simp [Image.read, named, data, ← mode]⟩

/-- Inversion of exactly one host acknowledgement. `Done` cannot come
from a partial write or an unexecuted barrier. -/
theorem consume {k w e rec r a log t n} (sel : selected k w rec = some r)
    (h : Walk k w e ((e, .Publish rec a) :: log) t n) :
    Enabled r a w.disk ∧ Walk k { w with disk := effect r a w.disk } .Done log t n := by
  cases h with | cons ans tail =>
    cases ans with | done act _ =>
      cases act with | publish sel' en =>
        rw [sel] at sel'
        cases sel'
        exact ⟨en, by simpa [Receipt.apply, execOps, Op.exec] using tail⟩

theorem publication_cut {k w e rec r log t n} (sel : selected k w rec = some r)
    (h : Walk k w e (pubSteps rec e .SyncNamespace ++ log) t n) :
    Enabled r .Stage w.disk ∧ Walk k { w with disk := published r w.disk } .Done log t n := by
  obtain ⟨fresh, h₁⟩ := consume sel h
  have keep (s : FS) : selected k {w with disk := s} rec = some r := by cases rec <;> exact sel
  obtain ⟨_, h₂⟩ := consume (keep (stage r w.disk)) h₁
  obtain ⟨_, h₃⟩ := consume (keep (sealed r w.disk)) h₂
  exact ⟨fresh, (consume (keep (renamed r w.disk)) h₃).2⟩

def initial (disk : FS) : World := ⟨disk, true, []⟩

theorem marker_at_cut {k disk log t n} (hm : Marked log) (h : Walk k (initial disk) .Start log t n) :
    ∀ f ∈ recordFacts k.marker, Fact.Stable f t.disk := by
  obtain ⟨rest, rfl⟩ := hm
  rw [List.append_assoc] at h
  obtain ⟨fresh, tail⟩ := publication_cut (rec := .Marker) (r := k.marker) rfl h
  cases tail with | cons ans afterReady =>
    cases ans with | done act _ =>
      cases act with | ready =>
        intro f hf
        apply afterReady.frame (Or.inr (Or.inl ⟨rfl, hf⟩))
        rcases (by simpa [recordFacts] using hf : f = _ ∨ f = _) with rfl | rfl
        · exact fun i hi => (checked_publication_record k.marker disk fresh i hi).1
        · exact fun i hi => (checked_publication_record k.marker disk fresh i hi).2

/-- A receipt for the last unit, tied to its capture and staged entry, not
some entry that happened to be readable. -/
theorem entry_at_cut {k w e f u x t} (h : Walk k w e (leafLog f u x .SyncNamespace) t .Done) :
    ∃ q ∈ t.history, Matches k q ∧ q.unit = u ∧ k.facts u = f ∧ q.before.live k = x ∧
      ∀ cell ∈ recordFacts q.record, Fact.Stable cell t.disk := by
  cases h with | cons a afterObserve =>
    cases a with | done act _ =>
      cases act with | observe =>
        cases afterObserve with | cons b afterCapture =>
          cases b with | done cap _ =>
            cases cap with | capture _ q ops m orig hc =>
              obtain ⟨fresh, finished⟩ := publication_cut (log := []) (rec := .Entry) (r := q.record) rfl afterCapture
              cases finished
              refine ⟨q, by simp [Receipt.apply], m, rfl, rfl, rfl, ?_⟩
              intro cell hm
              rcases (by simpa [recordFacts] using hm : cell = _ ∨ cell = _) with rfl | rfl
              · exact fun i hi => (checked_publication_record q.record _ fresh i hi).1
              · exact fun i hi => (checked_publication_record q.record _ fresh i hi).2

/-- Evidence a restarted reader can reach from the stable home. Inline
priors need only the decoded entry; file priors also need the named blob. -/
def Evidence (k : Context) (q : Packet) (u : U32) (i : recover.Intended) (image : Image) : Prop :=
  image.link k.journal.loc = some k.journal.inode ∧
  ((image.read k.marker.dst).bind (fun v => k.decodeMarker v.bytes)) = some (k.previous, k.target) ∧
  ((image.read q.record.dst).bind (fun v => k.decodeEntry v.bytes)) =
    some ⟨k.path u, q.original.ref k, q.before.ref k, i⟩ ∧
  (∀ fact ∈ q.original.facts ++ q.before.facts, fact.Holds image) ∧
  q.original.Recoverable k image ∧ q.before.Recoverable k image

/-- Filesystem-level P1. Quantifies over *all* interpreted executions reaching
this issued mutation of the translated Rust. The observed bytes decode to the
run's selections and this unit's path, original, immediate prior and intent.
Every required blob is reachable through its directory name with its bytes.
`CrashImage` includes both the visible (kill) and all allowed power images. -/
theorem mutation_evidence_survives {k : Context} {base disk s s' log e u d x i t}
    (parent : Prepare k.journal base disk) (run : Run s log)
    (effects : Walk k (initial disk) .Start log t e)
    (mutation : journal_run.run_step s e = ok (s', .Mutate u d x i)) :
    ∃ q ∈ t.history, Matches k q ∧ q.unit = u ∧ (k.facts u).dest = d ∧ q.before.live k = x ∧
      ∀ image, CrashImage t.disk image → Evidence k q u i image := by
  obtain ⟨rfl, hm, pre, facts, hl, hd, _, hi⟩ := mutate_after_durable_intent run mutation
  have marker := marker_at_cut hm effects
  have dir := effects.frame (Or.inl rfl) parent.durable
  have good := effects.saved (by simp [Saved, initial])
  have locations := effects.original_locations (by simp [initial])
  rw [hl] at effects
  obtain ⟨v, ev, _, leaf⟩ := effects.split
  obtain ⟨q, mem, m, rfl, rfl, hx, record⟩ := entry_at_cut leaf
  refine ⟨q, mem, m, rfl, hd, hx, ?_⟩
  intro img crash
  have readAt (r : Recipe) (hr : ∀ f ∈ recordFacts r, Fact.Stable f t.disk) : img.read r.dst = some r.value := by
    have hn := hr (.binding r.dst r.inode) (by simp [recordFacts]) img crash
    have hf := hr (.content r.inode r.value) (by simp [recordFacts]) img crash
    change img.link r.dst = some r.inode at hn
    change img.file r.inode = r.value at hf
    simp [Image.read, hn, hf]
  have evidence := fun f hf => good q mem f hf img crash
  refine ⟨dir _ crash, by simp [readAt _ marker, k.markerWire], ?_, evidence,
    Prior.recoverable (locations q mem) (fun f hf => evidence f (List.mem_append_left _ hf)),
    Prior.recoverable m.blob (fun f hf => evidence f (List.mem_append_right _ hf))⟩
  simp [readAt _ record, m.wire, m.path, hi]

/-- One packet works for both crash kinds. Kill releases the session but
a power cut, unlike kill, selects one of the pending persistence outcomes. -/
theorem mutation_cut_after_crash {k : Context} {base disk s s' log e u d x i t}
    (parent : Prepare k.journal base disk) (run : Run s log)
    (effects : Walk k (initial disk) .Start log t e)
    (mutation : journal_run.run_step s e = ok (s', .Mutate u d x i)) :
    ∃ q ∈ t.history, Matches k q ∧ q.unit = u ∧ (k.facts u).dest = d ∧ q.before.live k = x ∧
      ∀ (m : Machine) kind rest, m.fs = t.disk → Crash m kind rest →
        rest.session = none ∧ Evidence k q u i rest.fs.visible := by
  obtain ⟨q, mem, matched, unit, dest, expect, safe⟩ := mutation_evidence_survives parent run effects mutation
  refine ⟨q, mem, matched, unit, dest, expect, ?_⟩
  intro m kind rest same crash
  cases crash with
  | kill => exact ⟨rfl, safe _ (by rw [← same]; exact crash_visible m.fs)⟩
  | power hi => exact ⟨rfl, safe _ (same ▸ hi)⟩

/-- A failed stage/capture/barrier cannot issue a mutation (even though an
error may leave partial private effects). This is the shipped fail-closed
transition, not a convention that errors have no filesystem effects. -/
theorem failed_never_mutates (s : State) (failure : recover.Failure) :
    ∃ refused, journal_run.run_step s (.Failed failure) = ok ({s with phase := .Aborted}, .Abort refused) := by
  cases hp : s.phase <;> cases failure <;> cases hu : s.unit <;>
    simp [journal_run.run_step, hp, journal_run.failed_or_protocol, journal_run.on_act_failed, journal_run.refuse, hu]

end Gripsack.Protocol.Leaf
