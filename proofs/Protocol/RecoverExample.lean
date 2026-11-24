import Protocol.RecoveryCleanup

/-! One invocation, not independently reachable halves. The wire here is the
`journal_cleanup.rs` four-field test wire: the entry says absent prior, absent
immediate pre-state, intended removal. No selection read or hash computation
is needed for the markerless, all-absent path. Start with the destination
visibly unlinked but *not durably absent*; `Seal` checks it and syncs its
parent before the very same machine first unlinks an undo record. -/
namespace Gripsack.Protocol.RecoverExample

open Gripsack.Storage Gripsack.Protocol.RecoverLeaf Gripsack.Protocol.RecoverWalk
open Gripsack.Protocol.CleanupDisk Gripsack.Protocol.CleanupDisk.Example
open Gripsack.Protocol.RecoveryReplay Gripsack.Protocol.RecoveryTail
open Gripsack.Protocol.RecoveryCleanup
open Aeneas Aeneas.Std Result gripsack_kernel

/-- Not the historical ONE→TWO undo record. -/
def absentUndo : FileData := ⟨bytes "dest\n-\n-\n-", 0o600⟩

private def image : Image := ⟨fun l => if l = record then some 2 else if l = dest then some 4
    else if l = ⟨1,15⟩ then some 0 else none,
  fun n => if n = 2 then absentUndo else if n = 4 then two else ⟨[],0⟩⟩

def base : Machine := {start with fs := (FS.stable image).bind dest none}
def settled : Machine := {base with fs := base.fs.syncDir dest.parent}
private def dropped : Machine := {settled with fs := settled.fs.bind record none}
private def afterRecords : Machine := {dropped with fs := dropped.fs.syncDir marker.parent}
def finished : Machine := {afterRecords with fs := afterRecords.fs.syncDir marker.parent}

/-- The single frozen entry and a partial decoder of its actual private bytes.
The caller's codec is immaterial on absence; no collision law is synthesized. -/
def ctx (codec : Codec) : Context where
  codec := codec
  row _ := ⟨record,dest,absentLeaf,.absent,.absent⟩
  decodeEntry _ data := if data = absentUndo then some (absentLeaf,.absent) else none
  current := current
  pointer := "generations/2"
  manifest := manifest
  manifestInode := 6
  manifestData := selected
  decodeCurrent _ _ := none
  selectionChain := []
  selectionFacts := legacyOneJournal
  names := [record]
  count := rfl
  named i hi := by
    have z : i.val = 0 := by change i.val < 1 at hi; omega
    simp [z]

/-- Before cleanup the private record name and bytes are protected, even
though this permitted legacy invocation has no marker or selection chain. -/
def reserved (f : Leaf.Fact) := f = .binding record 2 ∨ f = .content 2 absentUndo

theorem names (codec) : layout.records = (ctx codec).names := rfl

theorem apart (codec) (i : U32) : ((ctx codec).row i).leaf ≠ marker ∧
    ((ctx codec).row i).leaf ∉ layout.records := by
  change dest ≠ marker ∧ dest ∉ [record]
  decide

theorem reserved_stable {f} (h : reserved f) : f.Stable base.fs := by
  rcases h with rfl | rfl
  · exact (show StableLink (FS.stable image) record (some 2) from
      fun _ hi => (crash_stable.mp hi) ▸ rfl).bind (by decide)
  · exact (show StableFile (FS.stable image) 2 absentUndo from
      fun _ hi => (crash_stable.mp hi) ▸ rfl).bind

theorem initially_volatile : ∃ i, CrashImage base.fs i ∧ i.link dest = some 4 :=
  ⟨image,crash_modify_skip (crash_visible (FS.stable image)),rfl⟩

private theorem checked : Seal dest .absent base settled :=
  .absent (.absent ⟨rfl,Or.inr rfl,rfl⟩ rfl) (.syncDir ⟨rfl,Or.inr rfl,rfl⟩)

private def quiet {k p m h} (read : Read k m p) (effect : Effect k m m p) :
    Answer k reserved h p m m :=
  ⟨read,effect,[],⟨rfl,fun i hi => ⟨i,hi,rfl⟩⟩,fun _ _ hk => hk,by simp,by simp⟩

/-- The exact exchanges of `legacy_seal_prefix`, now physically interpreted. -/
theorem replay (codec) : Replay (ctx codec) reserved base legacySealLog settled := by
  let k := ctx codec
  let readJ : Exchange := (.ReadJournal,.JournalRead legacyOneJournal)
  let readE : Exchange := (.ReadEntry 0#u32,.EntryRead absentLeaf)
  let observed : Exchange := (.Observe 0#u32 absentLeaf.dest,.Observed absentLeaf.dest .Absent)
  let ack : Exchange := (.Seal 0#u32 absentLeaf.dest .Absent .Absent,.Done)
  have first : Replay k reserved base [readJ] base :=
    .snoc .start (quiet rfl trivial)
  have decoded : Read k base readE := by
    refine ⟨rfl,?_⟩
    change (some absentUndo).bind (fun data => if data = absentUndo then _ else _) = _
    simp [k,ctx]
  have second : Replay k reserved base [readJ,readE] base := first.snoc (quiet decoded trivial)
  have third : Replay k reserved base [readJ,readE,observed] base :=
    second.snoc (quiet ⟨rfl,rfl⟩ trivial)
  have effect : Effect k base settled ack :=
    Reply.seal (actual := .absent) rfl rfl (fun _ => rfl) rfl checked
  apply third.snoc (p := ack)
  refine ⟨trivial,effect,[.syncDir dest.parent],⟨rfl,fun i hi => ⟨i,hi,rfl⟩⟩,
    fun _ _ hk => hk,?_,?_⟩
  · intro op ho f _
    obtain rfl := List.mem_singleton.mp ho
    cases f <;> trivial
  · intro z hz l v claimed
    rcases List.mem_cons.mp hz with rfl | hz
    · cases claimed
    rcases List.mem_cons.mp hz with rfl | hz
    · cases claimed
    obtain rfl := List.mem_singleton.mp hz
    cases claimed

private def syncingRecords := {legacySealState with index := 1#u32, phase := .SyncingRecords}
private def unlinkingMarker := {syncingRecords with phase := .UnlinkingMarker}
def syncingMarker := {unlinkingMarker with phase := .SyncingMarker}
def finalState := {syncingMarker with phase := .Finished}

theorem final_reply : recover.recover_step syncingMarker .Done = ok (finalState,.Finish .Recovered) := rfl

/-- The same call's outstanding unlink selects the real record name. There is
one checked unlink, a journal fsync, *pinned ENOENT*, then a second fsync. -/
theorem complete : Complete layout legacySealState (.Unlink (.Record 0#u32)) settled finished := by
  have add : U32.checked_add 0#u32 1#u32 = some 1#u32 := by
    have spec := U32.checked_add_bv_spec 0#u32 1#u32
    cases h : U32.checked_add 0#u32 1#u32 with
    | none => simp [h,U32.max_eq] at spec
    | some n => simp only [h] at spec; exact congrArg some (UScalar.eq_of_val_eq spec.2.1)
  have first : recover.recover_step legacySealState .Done = ok (syncingRecords,.Barrier .Journal) := by
    simp [legacySealState,recover.recover_step,add,lift,recover.next_unlink,syncingRecords]
  have un : Call layout settled (.record record) .ok dropped :=
    .io (.completed (.unlink (n := 2) (k := .regular) ⟨rfl,Or.inl rfl,rfl⟩ rfl rfl (by decide)))
  have sync₁ : Call layout dropped .sync .ok afterRecords :=
    .io (.completed (.syncDir ⟨rfl,Or.inl rfl,rfl⟩))
  have missing : Call layout afterRecords .marker .ok afterRecords :=
    .enoent ⟨rfl,Or.inl rfl,rfl⟩ rfl
  have sync₂ : Call layout afterRecords .sync .ok finished :=
    .io (.completed (.syncDir ⟨rfl,Or.inl rfl,rfl⟩))
  exact .reply rfl un first (.reply rfl sync₁ (t := unlinkingMarker) (d := .Unlink .Marker) rfl
    (.reply rfl missing (t := syncingMarker) (d := .Barrier .Journal) rfl
      (.reply rfl sync₂ final_reply .finished)))

/-- No marker decoder or selected-generation evidence is consumed on this path. -/
theorem admission (codec) (w : MarkerWire) : Admission (ctx codec) layout w base reserved legacySealState :=
  ⟨names codec,fun i _ _ => apart codec i,(fun h => by cases h),rfl⟩

/-- Apply the general cut theorem to this actual prefix and hand-off. This
also admits every possible interrupted suffix, not just our successful one. -/
theorem safe_at_cut (codec) (w : MarkerWire) {t}
    (cut : Execution layout legacySealState (.Unlink (.Record 0#u32)) settled t) :
    All t.fs (SafeImage (ctx codec) layout w legacySealState t.kind) :=
  cleanup_images legacy_seal_prefix trivial (replay codec) (admission codec w) cut

/-- One concrete run joins translated issuance, truthful replay, actual
`Finish Recovered` and the three durable absences (leaf, record, marker). -/
theorem reachable_leaf_cleanup (codec) :
    Prefix legacySealLog legacySealState (.Unlink (.Record 0#u32)) ∧
    Replay (ctx codec) reserved base legacySealLog settled ∧
    Complete layout legacySealState (.Unlink (.Record 0#u32)) settled finished ∧
    recover.recover_step syncingMarker .Done = ok (finalState,.Finish .Recovered) ∧
    StableLink finished.fs dest none ∧ StableLink finished.fs record none ∧ StableLink finished.fs marker none := by
  have clean := finish_retires_names legacy_seal_prefix trivial (replay codec) (names codec) complete
  have absent : StableLink settled.fs dest none := (StableLink.after_syncDir base.fs dest)
  exact ⟨legacy_seal_prefix,replay codec,complete,final_reply,(absent.bind (by decide)).syncDir.syncDir,
    clean.2 record (by simp [layout]),clean.1⟩

end Gripsack.Protocol.RecoverExample
