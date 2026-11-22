import Protocol.RecoverWalk
import Protocol.RecoveryReplay
import Protocol.RecoveryTail

/-! # Leaf P3: retiring a journal record at an interpreted recovery cut

The prefix comes from the translated Rust, the replies from checked storage, and
only the command that Rust actually issued selects an unlink or fsync. No
premise says that all leaves settled, that cleanup was successful, or that a
foreign observation was durable. The private wire reader, complete enumeration,
reserved ancestor/manifest bytes and exclusion of later aliasing/external edits
are explicit boundaries. Directory callbacks are not proved here (premise D).
-/
namespace Gripsack.Protocol.RecoveryCleanup

open Gripsack.Storage Gripsack.Protocol.RecoverLeaf Gripsack.Protocol.RecoveryReplay
open Gripsack.Protocol.RecoverWalk Gripsack.Protocol.CleanupDisk Gripsack.Protocol.RecoveryTail
open Aeneas.Std Aeneas.Std.Result gripsack_kernel

structure MarkerWire where
  directory : Loc
  inode : Nat
  data : FileData
  decode : FileData → Option recover.MarkerFacts

def Outside (j : Layout) (l : Loc) := l ≠ j.marker ∧ l ∉ j.records

def SurvivesNames (j : Layout) : Leaf.Fact → Prop
  | .binding l _ => Outside j l
  | .content .. => True

def Source (base : Machine) (reserved : Leaf.Fact → Prop) (f : Leaf.Fact) :=
  reserved f ∧ f.Stable base.fs

/-- Private sources, established before this recovery. A selected manifest and
its ancestors have their own independent publication contract; syncing `current`'s
parent does not write them. The marked branch starts with durable marker evidence
(as supplied by leaf P1). Restart during cleanup is handled by `restart_cut_marked`,
which also covers original entries merely visibly absent when rescanned after kill. -/
structure CommittedInputs (k : Context) (j : Layout) (w : MarkerWire) (base : Machine)
    (reserved : Leaf.Fact → Prop) (mk : recover.MarkerFacts) : Prop where
  markerName : Source base reserved (.binding j.marker w.inode)
  markerBytes : Source base reserved (.content w.inode w.data)
  markerWire : w.decode w.data = some mk
  journalName : Source base reserved (.binding w.directory j.marker.parent)
  journalAway : Outside j w.directory
  currentAway : Outside j k.current
  manifestName : Source base reserved (.binding k.manifest k.manifestInode)
  manifestBytes : Source base reserved (.content k.manifestInode k.manifestData)
  manifestAway : Outside j k.manifest
  chain : ∀ f ∈ k.selectionChain, Source base reserved f ∧ SurvivesNames j f

/-- Non-delegated leaf names cannot alias names in the private journal. An
ordinal is resolved with the same pinned `record_name` enumeration as `ReadEntry`.
The committed-data premise applies only when the current reader actually
classified the run as committed; no selected manifest is demanded of a fresh run. -/
structure Admission (k : Context) (j : Layout) (w : MarkerWire) (base : Machine)
    (reserved : Leaf.Fact → Prop) (s : recover.RecoverState) : Prop where
  inventory : j.records = k.names
  privateJournal : ∀ i : U32, i.val < k.selectionFacts.entries.val → (k.row i).facts.route = .Leaf →
    Outside j (k.row i).leaf
  committed : s.verdict = .Committed → ∀ mk, s.journal.marker = some mk → CommittedInputs k j w base reserved mk
  header : match k.selectionFacts.marker with
    | none => base.fs.visible.link j.marker = none
    | some mk => (base.fs.visible.read j.marker).bind w.decode = some mk

/-- Missing is an actual absence, never a swallowed read or decode failure.
   Markerless directory runs are not admitted as legacy leaves. -/
theorem legacy_handoff {k j w base reserved h disk s c} (p : Prefix h s c) (start : IsCleanup c)
    (r : Replay k reserved base h disk) (a : Admission k j w base reserved s)
    (missing : s.journal.marker = none) :
    base.fs.visible.link j.marker = none ∧ s.journal.directories = 0#u32 ∧ s.verdict = .Uncommitted := by
  obtain ⟨⟨journal,_⟩,origin⟩ := handoff_origin p start
  obtain ⟨_,_,read,_⟩ := r.answered journal
  change s.journal = k.selectionFacts at read
  have honest := a.header
  rw [← read,missing] at honest
  rcases handoff p start with ⟨_,has,_⟩ | ⟨uncommitted,_⟩
  · simp [missing] at has
  · rw [uncommitted] at origin
    rcases origin with ⟨_,noDirs⟩ | ⟨m,found,_⟩
    · exact ⟨honest,noDirs,uncommitted⟩
    · simp [missing] at found

private theorem cursor {k j base reserved h disk s c} (p : Prefix h s c) (start : IsCleanup c)
    (replay : Replay k reserved base h disk) (names : j.records = k.names) : Remaining j s c j.plan := by
  obtain ⟨jread,_⟩ := (handoff_origin p start).1
  obtain ⟨_,_,read,_⟩ := replay.answered jread
  change s.journal = k.selectionFacts at read
  obtain ⟨zero,count,branches⟩ := cleanup_start p start
  have len : s.records.val = (j.records.map Op.record).length := by
    rw [List.length_map,names,k.count,←read,count]
  rcases branches with short | exhausted
  · obtain ⟨within,phase,rfl⟩ := short
    simpa [Layout.plan,zero] using (CleanupOrder.Cursor.record (records := j.records.map Op.record)
      (sync := Op.sync) (marker := Op.marker) phase len within)
  · obtain ⟨done_,phase,rfl⟩ := exhausted
    have empty : j.records = [] := List.length_eq_zero_iff.mp (by simpa [zero,len] using done_)
    simpa [Remaining,Layout.plan,empty] using (CleanupOrder.Cursor.first (records := j.records.map Op.record)
      (sync := Op.sync) (marker := Op.marker) phase)

private theorem cut_durable {j m t l v} (cut : Cut j m j.plan t) (away : Outside j l)
    (stable : Durable m l v) : Durable t l v :=
  stable.transport (fun _ _ hk => cut.kind ▸ hk) (fun _ => cut.frameLink away.1 away.2)
    (fun _ _ _ => cut.frameFile)

private theorem cut_fact {j m t f} (cut : Cut j m j.plan t) (apart : SurvivesNames j f)
    (stable : f.Stable m.fs) : f.Stable t.fs := by
  cases f with
  | binding => exact cut.frameLink apart.1 apart.2 stable
  | content => exact cut.frameFile stable

private theorem uncommitted_leaf {k base reserved h disk} (r : Replay k reserved base h disk)
    {i : U32} (walk : Processed h i) (leaf : (k.row i).facts.route = .Leaf) :
    Durable disk (k.row i).leaf (k.row i).prior ∨
    (recover.decide (k.row i).facts ((k.row i).observed.live k.codec) = ok .Preserve ∧
      Durable disk (k.row i).leaf (k.row i).observed) := by
  obtain ⟨f,read,route⟩ := walk
  obtain ⟨_,_,readAt,_⟩ := r.answered read
  have eq : f = (k.row i).facts := readAt.1
  subst f
  rcases route with delegated | ⟨_,seen,_observation,settled⟩
  · rw [leaf] at delegated; cases delegated
  rcases settled with ⟨landed,ack,_prior⟩ | sealedLeaf | kept
  · exact Or.inl (r.claimed ack rfl)
  · exact Or.inl (r.claimed sealedLeaf rfl)
  · obtain ⟨_,_,_,effect⟩ := r.answered kept
    exact Or.inr ⟨effect.preserved,r.claimed kept rfl⟩

/-- A marked restart can read the original marker and classify the committed
target, or every old record name is absent. No permission to undo committed
bytes comes from a generation number. In the uncommitted (including legitimate
markerless) case *every admitted leaf* is already at its prior or at the exact
foreign observation whose real `Preserve` reply sealed its bytes and binding. -/
def SafeImage (k : Context) (j : Layout) (w : MarkerWire) (s : recover.RecoverState)
    (kind : Nat → Option Kind) (img : Image) : Prop :=
  (s.verdict = .Committed ∧ ∃ mk, s.journal.marker = some mk ∧ s.current = some mk.target ∧
    Holds kind img k.current (.link k.pointer) ∧
    (img.read k.manifest).bind (k.decodeCurrent k.pointer) = some mk.target ∧
    (∀ f ∈ k.selectionChain, f.Holds img) ∧ img.link w.directory = some j.marker.parent ∧
    ((img.read j.marker).bind w.decode = some mk ∨ ∀ l ∈ j.records, img.link l = none)) ∨
  (s.verdict = .Uncommitted ∧ ∀ i : U32, i.val < s.journal.entries.val → (k.row i).facts.route = .Leaf →
    Holds kind img (k.row i).leaf (k.row i).prior ∨
    (recover.decide (k.row i).facts ((k.row i).observed.live k.codec) = ok .Preserve ∧
      Holds kind img (k.row i).leaf (k.row i).observed))

private theorem named_data {i : Image} {l n data} (hn : i.link l = some n) (hf : i.file n = data) :
    i.read l = some data := by simp [Image.read,hn,hf]

theorem cleanup_images {k j w base reserved h disk s c t} (p : Prefix h s c) (start : IsCleanup c)
    (r : Replay k reserved base h disk) (basis : Admission k j w base reserved s)
    (run : Execution j s c disk t) : All t.fs (SafeImage k j w s t.kind) := by
  have cut := run.cut (cursor p start r basis.inventory)
  have origin := handoff_origin p start
  obtain ⟨_,_,jread,_⟩ := r.answered origin.1.1
  change s.journal = k.selectionFacts at jread
  rcases handoff p start with ⟨vc,_,bar⟩ | ⟨vu,covered⟩
  · have classified := origin.2
    rw [vc] at classified
    obtain ⟨mk,hmark,hcur,verdict⟩ := classified
    have current : s.current = some mk.target :=
      (Kernel.spec_eq_ok (Kernel.classify_committed_iff _) verdict).mp rfl
    have a := basis.committed vc mk hmark
    rw [current] at hcur
    obtain ⟨_,before,_,pre,hread,_⟩ := r.at_answer hcur
    have man := named_data ((pre.keeps a.manifestName.1 a.manifestName.2) _ (crash_visible before.fs))
      ((pre.keeps a.manifestBytes.1 a.manifestBytes.2) _ (crash_visible before.fs))
    have decoded := hread.2.1
    rw [man] at decoded
    have pointer : Durable t k.current (.link k.pointer) := cut_durable cut a.currentAway (r.claimed bar rfl)
    have names := cut.marked (r.keeps a.markerName.1 a.markerName.2)
    intro img crash
    refine Or.inl ⟨vc,mk,hmark,current,pointer.holds _ crash,?_,?_,?_,?_⟩
    · rw [named_data ((cut.frameLink a.manifestAway.1 a.manifestAway.2 (r.keeps a.manifestName.1 a.manifestName.2)) img crash)
        ((cut.frameFile (r.keeps a.manifestBytes.1 a.manifestBytes.2)) img crash)]
      exact decoded
    · intro f hf
      have ⟨⟨keep,stable⟩,apart⟩ := a.chain f hf
      exact cut_fact cut apart (r.keeps keep stable) img crash
    · exact cut.frameLink a.journalAway.1 a.journalAway.2 (r.keeps a.journalName.1 a.journalName.2) img crash
    · rcases names img crash with mark | noRecords
      · left; rw [named_data mark ((cut.frameFile (r.keeps a.markerBytes.1 a.markerBytes.2)) img crash)]
        exact a.markerWire
      · exact Or.inr noRecords
  · intro img crash
    refine Or.inr ⟨vu,fun i within leaf => ?_⟩
    have away := basis.privateJournal i (jread ▸ within) leaf
    rcases uncommitted_leaf r (covered i within) leaf with good | ⟨foreign,good⟩
    · exact Or.inl ((cut_durable cut away good).holds _ crash)
    · exact Or.inr ⟨foreign,(cut_durable cut away good).holds _ crash⟩

/-- Leaf P3 at every stopped or completed cleanup cut, including an effect that
happened before an error/lost reply. A kill retains all the outstanding power
choices; a power cut selects one of them. Neither makes a failed barrier succeed. -/
theorem leaf_cleanup_after_crash {k j w base reserved h disk s c t restart cause}
    (p : Prefix h s c) (start : IsCleanup c) (r : Replay k reserved base h disk)
    (basis : Admission k j w base reserved s) (run : Execution j s c disk t)
    (crash : Crash t cause restart) : restart.session = none ∧ All restart.fs (SafeImage k j w s restart.kind) := by
  have all := restart_all crash (cleanup_images p start r basis run)
  cases crash <;> exact ⟨rfl,all⟩

/-- Successful return means that neither the marker nor an admitted record can
return on restart. The already-absent marker is not an extra success assumption. -/
theorem finish_retires_names {k j base reserved h disk s c t} (p : Prefix h s c) (start : IsCleanup c)
    (r : Replay k reserved base h disk) (names : j.records = k.names) (finished : Complete j s c disk t) :
    StableLink t.fs j.marker none ∧ ∀ l ∈ j.records, StableLink t.fs l none :=
  (finished.walk (cursor p start r names)).clean

end Gripsack.Protocol.RecoveryCleanup
