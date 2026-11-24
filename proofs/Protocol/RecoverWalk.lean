import Kernel.Recover

/-! # Admission and leaf traversal up to the first cleanup command

`Prefix` records completed command/answer pairs in driver order, not a set of
assumed per-entry obligations. The command at the right boundary is still
outstanding. Stop *before* answering the first cleanup command; its execution
and subsequent crashes belong to the storage interpretation.
-/
open Aeneas Aeneas.Std Result gripsack_kernel Gripsack.Kernel

namespace Gripsack.Protocol.RecoverWalk

abbrev Log := List (recover.Command × recover.Event)

/-- Responses the driver can return. `Preserve` has no expected-live argument;
its adapter must consult the stored observation. A failure is not a `Done`. -/
def Answers : recover.Command → recover.Event → Prop
  | .Abort _, _ | .Finish _, _ => False
  | _, .Failed _ => True
  | .ReadJournal, .NoJournal | .ReadJournal, .JournalRead _ => True
  | .ReadCurrent, .CurrentRead _ | .ReadEntry _, .EntryRead _ => True
  | .Observe _ d, .Observed d' _ => d = d'
  | .Mutate .., .Landed _ => True
  | .Seal .., .Done | .Preserve .., .Done | .Delegate _, .Done
  | .Barrier _, .Done | .Unlink _, .Done => True
  | _, _ => False

def IsCleanup : recover.Command → Prop
  | .Unlink (.Record _) | .Barrier .Journal => True
  | _ => False

inductive Prefix : Log → recover.RecoverState → recover.Command → Prop where
  | start {initial s c} (new : recover.RecoverState.new = ok initial)
      (step : recover.recover_step initial .Start = ok (s, c)) : Prefix [] s c
  | answer {h s c e t d} (prev : Prefix h s c) (before : ¬ IsCleanup c)
      (reply : Answers c e) (step : recover.recover_step s e = ok (t,d)) :
      Prefix (h ++ [(c,e)]) t d

def Matches (p : recover.Prior) (l : recover.Live) : Prop :=
  (p = .Absent ∧ l = .Absent) ∨ ∃ ident, p = .Obj ident ∧ l = .Obj ident

def Settled (h : Log) (i : U32) (f : recover.EntryFacts) (seen : recover.Live) : Prop :=
  (∃ landed, (.Mutate i f.dest seen (.Restore f.prior), .Landed landed) ∈ h ∧
    Matches f.prior landed) ∨
  (.Seal i f.dest seen f.prior, .Done) ∈ h ∨ (.Preserve i f.dest, .Done) ∈ h

def Processed (h : Log) (i : U32) : Prop :=
  ∃ f, (.ReadEntry i, .EntryRead f) ∈ h ∧ (f.route = .Delegated ∨
    f.route = .Leaf ∧ ∃ seen, (.Observe i f.dest, .Observed f.dest seen) ∈ h ∧ Settled h i f seen)

def Covered (h : Log) (n : Nat) := ∀ i : U32, i.val < n → Processed h i

def Certificate (h : Log) (s : recover.RecoverState) : Prop :=
  (s.verdict = .Committed ∧ s.journal.marker.isSome ∧ (.Barrier .Selection,.Done) ∈ h) ∨
  (s.verdict = .Uncommitted ∧ Covered h s.journal.entries.val)

/-- The exact journal read (including marker), checked record count, and
current-pointer read used for classification, or the permitted legacy route. -/
def Known (h : Log) (j : recover.JournalFacts) (r : U32) : Prop :=
  (.ReadJournal,.JournalRead j) ∈ h ∧ r.val = j.entries.val + j.directories.val

def ClassifiedAs (h : Log) (s : recover.RecoverState) (k : selection.Classification) : Prop :=
  ∃ m, s.journal.marker = some m ∧ (.ReadCurrent,.CurrentRead s.current) ∈ h ∧
    selection.classify ⟨m.previous,m.target,s.current⟩ = ok k

def Origin (h : Log) (s : recover.RecoverState) : Prop :=
  Known h s.journal s.records ∧ match s.verdict with
  | .Committed => ClassifiedAs h s .Committed
  | .Uncommitted => (s.journal.marker = none ∧ s.journal.directories = 0#u32) ∨
                     ClassifiedAs h s .Uncommitted
  | .Pending => False

def Begins (s : recover.RecoverState) (c : recover.Command) : Prop :=
  s.index.val = 0 ∧ s.records.val = s.journal.entries.val + s.journal.directories.val ∧
  ((s.index.val < s.records.val ∧ s.phase = .UnlinkingRecord ∧ c = .Unlink (.Record s.index)) ∨
   (s.records.val ≤ s.index.val ∧ s.phase = .SyncingRecords ∧ c = .Barrier .Journal))

private theorem processed_mono {h : Log} {i} (p : Processed h i) (x) : Processed (h ++ [x]) i := by
  rcases p with ⟨f,hr,hd | ⟨hl,seen,ho,hs⟩⟩
  · exact ⟨f,List.mem_append_left _ hr,Or.inl hd⟩
  · refine ⟨f,List.mem_append_left _ hr,Or.inr ⟨hl,seen,List.mem_append_left _ ho,?_⟩⟩
    rcases hs with ⟨l,hm,hp⟩ | hs | hp
    · exact Or.inl ⟨l,List.mem_append_left _ hm,hp⟩
    · exact Or.inr (Or.inl (List.mem_append_left _ hs))
    · exact Or.inr (Or.inr (List.mem_append_left _ hp))

private theorem origin_mono {h s} (p : Origin h s) (x) : Origin (h ++ [x]) s := by
  obtain ⟨⟨hr,hc⟩,hv⟩ := p
  refine ⟨⟨List.mem_append_left _ hr,hc⟩,?_⟩
  cases he : s.verdict <;> simp only [he] at hv ⊢
  · obtain ⟨m,hm,hr,hc⟩ := hv
    exact ⟨m,hm,List.mem_append_left _ hr,hc⟩
  · rcases hv with h | ⟨m,hm,hr,hc⟩
    · exact Or.inl h
    · exact Or.inr ⟨m,hm,List.mem_append_left _ hr,hc⟩

private abbrev Running (h : Log) (s : recover.RecoverState) :=
  Origin h s ∧ s.verdict = .Uncommitted ∧ Covered h s.index.val

private theorem running_mono {h s} (r : Running h s) (x) : Running (h ++ [x]) s :=
  ⟨origin_mono r.1 x,r.2.1,fun i hi => processed_mono (r.2.2 i hi) x⟩

private abbrev ReadLeaf (h : Log) (s : recover.RecoverState) (f : recover.EntryFacts) :=
  s.entry = some f ∧ f.route = .Leaf ∧ (.ReadEntry s.index,.EntryRead f) ∈ h

private abbrev Seen (h : Log) (s : recover.RecoverState) (f : recover.EntryFacts) (l : recover.Live) :=
  ReadLeaf h s f ∧ (.Observe s.index f.dest,.Observed f.dest l) ∈ h

private abbrev Ready (h : Log) (s : recover.RecoverState) := Origin h s ∧ Certificate h s

private def Stage (h : Log) (s : recover.RecoverState) (c : recover.Command) : Prop :=
  match c with
  | .ReadJournal => s.phase = .ReadingJournal
  | .ReadCurrent => s.phase = .ReadingCurrent ∧ Known h s.journal s.records
  | .Barrier .Selection => s.phase = .SealingSelection ∧ Origin h s ∧ s.verdict = .Committed
  | .Delegate .Committed => s.phase = .SettlingDirectories ∧ Ready h s
  | .Delegate .BeforeLeaves => s.phase = .OpeningLeaves ∧ Running h s
  | .Delegate .AfterLeaves => s.phase = .ClosingLeaves ∧ Ready h s
  | .ReadEntry i => s.phase = .ReadingEntry ∧ Running h s ∧ i = s.index
  | .Observe i d => s.phase = .Observing ∧ Running h s ∧ i = s.index ∧
      ∃ f, ReadLeaf h s f ∧ d = f.dest
  | .Mutate i d seen (.Restore p) => s.phase = .Restoring ∧ Running h s ∧ i = s.index ∧
      ∃ f, Seen h s f seen ∧ d = f.dest ∧ p = f.prior
  | .Seal i d seen p => s.phase = .Sealing ∧ Running h s ∧ i = s.index ∧
      ∃ f, Seen h s f seen ∧ d = f.dest ∧ p = f.prior
  | .Preserve i d => s.phase = .Preserving ∧ Running h s ∧ i = s.index ∧
      ∃ f seen, Seen h s f seen ∧ d = f.dest
  | .Unlink (.Record _) | .Barrier .Journal => Ready h s ∧ Begins s c
  | .Unlink .Marker => False
  | .Abort _ | .Finish _ => True

private theorem ready_mono {h s} (r : Ready h s) (x) : Ready (h ++ [x]) s := by
  refine ⟨origin_mono r.1 x,?_⟩
  rcases r.2 with ⟨hv,hm,hd⟩ | ⟨hv,hc⟩
  · exact Or.inl ⟨hv,hm,List.mem_append_left _ hd⟩
  · exact Or.inr ⟨hv,fun i hi => processed_mono (hc i hi) x⟩

private theorem begin_stage {h s} (r : Ready h s) :
    recover.begin_cleanup s ⦃ p => Stage h p.1 p.2 ⦄ := by
  apply WP.spec_mono (begin_cleanup_spec _)
  rintro p (⟨hlt,rfl⟩ | ⟨hle,rfl⟩)
  · exact ⟨r,rfl,r.1.1.2,Or.inl ⟨hlt,rfl,rfl⟩⟩
  · exact ⟨r,rfl,r.1.1.2,Or.inr ⟨hle,rfl,rfl⟩⟩

private theorem loop_ready {h s} (r : Running h s) (endEntries : ¬ s.index < s.journal.entries) :
    Ready h s := by
  have bound : s.journal.entries.val ≤ s.index.val := by scalar_tac
  exact ⟨r.1,Or.inr ⟨r.2.1,fun i hi => r.2.2 i (by omega)⟩⟩

/-- The shipped `proceed_spec` forgets the branch predicates. Recover the
exhaustion bound at both paths to retirement, from the actual Rust branch. -/
private theorem proceed_stage {h s} (r : Running h s) :
    recover.proceed s ⦃ p => Stage h p.1 p.2 ⦄ := by
  unfold recover.proceed
  split_ifs <;> first
    | exact begin_stage (loop_ready r (by assumption))
    | (simp only [WP.spec_ok]; exact ⟨rfl,loop_ready r (by assumption)⟩)
    | (simp only [WP.spec_ok]; exact ⟨rfl,r,rfl⟩)
    | (simp only [WP.spec_ok]; exact ⟨rfl,r⟩)

private theorem advance_stage {h s} (r : Running h s) (last : Processed h s.index) :
    recover.advance s ⦃ p => Stage h p.1 p.2 ⦄ := by
  unfold recover.advance
  have add := U32.checked_add_bv_spec s.index 1#u32
  cases ho : U32.checked_add s.index 1#u32 with
  | none => simp [lift,recover.refuse,Stage]
  | some i =>
    simp only [ho] at add
    simp only [lift,Std.bind_ok]
    apply proceed_stage
    refine ⟨r.1,r.2.1,?_⟩
    have hi : i.val = s.index.val + 1 := by simpa using add.2.1
    intro k hk
    change k.val < i.val at hk
    by_cases before : k.val < s.index.val
    · exact r.2.2 k before
    · have eq : k = s.index := UScalar.eq_of_val_eq (by omega)
      exact eq ▸ last

private theorem journal_stage {h s} (j : recover.JournalFacts) :
    recover.on_journal s j ⦃ p => Stage (h ++ [(.ReadJournal,.JournalRead j)]) p.1 p.2 ⦄ := by
  unfold recover.on_journal
  split
  · simp [recover.refuse,Stage]
  · have add := U32.checked_add_bv_spec j.entries j.directories
    cases ho : U32.checked_add j.entries j.directories with
    | none => simp [lift,recover.refuse,Stage]
    | some r =>
      simp only [ho] at add
      simp only [lift,Std.bind_ok]
      have known : Known (h ++ [(.ReadJournal,.JournalRead j)]) j r := ⟨by simp,add.2.1⟩
      cases hm : j.marker with
      | some m => exact (WP.spec_ok _).mpr ⟨rfl,known⟩
      | none =>
        simp only
        split_ifs with hd
        · simp [recover.refuse,Stage]
        · apply proceed_stage
          exact ⟨⟨known,Or.inl ⟨hm,UScalar.eq_of_val_eq (by simpa using hd)⟩⟩,rfl,by intro i hi; simp at hi⟩

private theorem current_stage {h s} (k : Known h s.journal s.records)
    (cur : Option selection.SelectionIdentity) :
    recover.on_current s cur ⦃ p => Stage (h ++ [(.ReadCurrent,.CurrentRead cur)]) p.1 p.2 ⦄ := by
  unfold recover.on_current
  cases hm : s.journal.marker with
  | none => simp [recover.refuse,Stage]
  | some m =>
    obtain ⟨c,hc,_⟩ := spec_exists_ok (classify_committed_iff ⟨m.previous,m.target,cur⟩)
    simp only
    rw [hc,Std.bind_ok]
    have known : Known (h ++ [(.ReadCurrent,.CurrentRead cur)]) s.journal s.records :=
      ⟨List.mem_append_left _ k.1,k.2⟩
    cases c with
    | Committed => exact (WP.spec_ok _).mpr ⟨rfl,⟨known,m,hm,by simp,hc⟩,rfl⟩
    | Uncommitted =>
      apply proceed_stage
      exact ⟨⟨known,Or.inr ⟨m,hm,by simp,hc⟩⟩,rfl,by intro i hi; simp at hi⟩
    | Ambiguous => simp [recover.refuse,Stage]

private theorem observed_stage {h s f} (r : Running h s) (read : ReadLeaf h s f)
    (seen : recover.Live) :
    recover.on_observed s f.dest seen ⦃ p =>
      Stage (h ++ [(.Observe s.index f.dest,.Observed f.dest seen)]) p.1 p.2 ⦄ := by
  apply WP.spec_mono (on_observed_spec _ _ _)
  rintro p (⟨_,rfl,_⟩ | ⟨g,hg,_,_,branches⟩)
  · trivial
  · have eq := Option.some.inj (read.1.symm.trans hg)
    subst g
    have obs : Seen (h ++ [(.Observe s.index f.dest,.Observed f.dest seen)]) s f seen :=
      ⟨⟨read.1,read.2.1,List.mem_append_left _ read.2.2⟩,by simp⟩
    rcases branches with ⟨_,rfl⟩ | rfl | rfl
    · exact ⟨rfl,running_mono r _,rfl,f,obs,rfl,rfl⟩
    · exact ⟨rfl,running_mono r _,rfl,f,obs,rfl,rfl⟩
    · exact ⟨rfl,running_mono r _,rfl,f,seen,obs,rfl⟩

private theorem leaf_processed {h s f l} (obs : Seen h s f l) (x)
    (done : Settled (h ++ [x]) s.index f l) : Processed (h ++ [x]) s.index :=
  ⟨f,List.mem_append_left _ obs.1.2.2,Or.inr ⟨obs.1.2.1,l,List.mem_append_left _ obs.2,done⟩⟩

private theorem landed_stage {h s f seen} (r : Running h s) (obs : Seen h s f seen)
    (l : recover.Live) :
    recover.on_landed s l ⦃ p => Stage
      (h ++ [(.Mutate s.index f.dest seen (.Restore f.prior),.Landed l)]) p.1 p.2 ⦄ := by
  unfold recover.on_landed
  rw [obs.1.1]
  simp only
  step with Prior.matches_spec as ⟨b,hb⟩
  split_ifs with yes
  · exact advance_stage (running_mono r _) (leaf_processed obs _ (Or.inl ⟨l,by simp,hb.mp yes⟩))
  · simp [recover.refuse,Stage]

/-- A failed answer never dispatches even a single subsequent destination
command (restore and seal have their own drift diagnostic). This says nothing
about driver-side quarantine/diagnostics on the failing call itself. -/
theorem failure_aborts (s : recover.RecoverState) (f : recover.Failure) :
    recover.recover_step s (.Failed f) ⦃ p => ∃ r, p.2 = .Abort r ⦄ := by
  cases hp : s.phase <;> simp only [recover.recover_step,hp] <;> first
    | exact WP.spec_mono (failed_or_protocol_spec _ _) (by rintro p ⟨r,rfl⟩; exact ⟨r,rfl⟩)
    | exact WP.spec_mono (on_restore_failed_spec _ _) (by rintro p ⟨r,rfl⟩; exact ⟨r,rfl⟩)
    | simp [recover.refuse]

set_option maxHeartbeats 1000000 in
private theorem step_stage {h s c e} (st : Stage h s c) (pre : ¬ IsCleanup c) (a : Answers c e) :
    recover.recover_step s e ⦃ p => Stage (h ++ [(c,e)]) p.1 p.2 ⦄ := by
  cases c <;> cases e
  all_goals try exact False.elim a
  all_goals try exact WP.spec_mono (failure_aborts _ _) (by rintro p ⟨r,hc⟩; rw [hc]; trivial)
  case ReadJournal.NoJournal =>
    simp [recover.recover_step,show s.phase = .ReadingJournal from st,Stage]
  case ReadJournal.JournalRead j =>
    simpa only [recover.recover_step,show s.phase = .ReadingJournal from st] using
      journal_stage (s := s) (h := h) j
  case ReadCurrent.CurrentRead cur =>
    obtain ⟨hp,hk⟩ := st
    simpa only [recover.recover_step,hp] using current_stage hk cur
  case ReadEntry.EntryRead i f =>
    obtain ⟨hp,r,rfl⟩ := st
    simp only [recover.recover_step,hp,recover.on_entry]
    cases hr : f.route with
    | Delegated => exact advance_stage (running_mono r _) ⟨f,by simp,Or.inl hr⟩
    | Leaf =>
      exact (WP.spec_ok _).mpr ⟨rfl,running_mono r _,rfl,f,⟨rfl,hr,by simp⟩,rfl⟩
  case Observe.Observed i d d' l =>
    obtain ⟨hp,r,rfl,f,read,rfl⟩ := st
    change f.dest = d' at a
    subst d'
    simpa only [recover.recover_step,hp] using observed_stage r read l
  case Mutate.Landed i d seen eff l =>
    cases eff with | Restore p =>
      obtain ⟨hp,r,rfl,f,obs,rfl,rfl⟩ := st
      simpa only [recover.recover_step,hp] using landed_stage r obs l
  case Seal.Done i d seen p =>
    obtain ⟨hp,r,rfl,f,obs,rfl,rfl⟩ := st
    simp only [recover.recover_step,hp]
    exact advance_stage (running_mono r _) (leaf_processed obs _ (Or.inr (Or.inl (by simp))))
  case Preserve.Done i d =>
    obtain ⟨hp,r,rfl,f,seen,obs,rfl⟩ := st
    simp only [recover.recover_step,hp]
    exact advance_stage (running_mono r _) (leaf_processed obs _ (Or.inr (Or.inr (by simp))))
  case Delegate.Done d =>
    cases d with
    | Committed | AfterLeaves =>
      obtain ⟨hp,r⟩ := st
      simp only [recover.recover_step,hp]
      exact begin_stage (ready_mono r _)
    | BeforeLeaves =>
      obtain ⟨hp,r⟩ := st
      simp only [recover.recover_step,hp]
      exact proceed_stage (running_mono r _)
  case Barrier.Done b =>
    cases b with
    | Journal => exact False.elim (pre trivial)
    | Selection =>
      obtain ⟨hp,o,hv⟩ := st
      have source := o.2
      rw [hv] at source
      obtain ⟨m,hm,_⟩ := source
      have ready : Ready (h ++ [(.Barrier .Selection,.Done)]) s :=
        ⟨origin_mono o _,Or.inl ⟨hv,by simp [hm],by simp⟩⟩
      simp only [recover.recover_step,hp]
      split
      · exact (WP.spec_ok _).mpr ⟨rfl,ready⟩
      · exact begin_stage ready
  case Unlink.Done n =>
    cases n with
    | Marker => exact False.elim st
    | Record i => exact False.elim (pre trivial)

private theorem prefix_stage {h s c} (p : Prefix h s c) : Stage h s c := by
  induction p with
  | start new hs =>
    simp only [recover.RecoverState.new,Result.ok.injEq] at new
    subst_vars
    simp only [recover.recover_step,Result.ok.injEq,Prod.mk.injEq] at hs
    obtain ⟨rfl,rfl⟩ := hs
    rfl
  | answer p before reply hs ih =>
    have post := step_stage ih before reply
    rw [hs] at post
    simpa only [WP.spec_ok] using post

private theorem cleanup_stage {h s c} (p : Prefix h s c) (hc : IsCleanup c) :
    Ready h s ∧ Begins s c := by
  have st := prefix_stage p
  cases c <;> try contradiction
  case Unlink n => cases n <;> first | exact st | contradiction
  case Barrier b => cases b <;> first | exact st | contradiction

theorem handoff {h s c} (p : Prefix h s c) (hc : IsCleanup c) : Certificate h s :=
  (cleanup_stage p hc).1.2

theorem handoff_origin {h s c} (p : Prefix h s c) (hc : IsCleanup c) : Origin h s :=
  (cleanup_stage p hc).1.1

theorem cleanup_start {h s c} (p : Prefix h s c) (hc : IsCleanup c) : Begins s c :=
  (cleanup_stage p hc).2

/-- An ill-formed decoded header retires no evidence and issues no next
destination command. An undecodable/quarantined read may instead return
`Failed`; `failure_aborts` covers it regardless of the driver's diagnostics. -/
theorem malformed_header_aborts (s : recover.RecoverState) (atRead : s.phase = .ReadingJournal)
    (j : recover.JournalFacts)
    (bad : j.reverse_roots > j.entries ∨ U32.checked_add j.entries j.directories = none ∨
      (j.marker = none ∧ j.directories ≠ 0#u32)) :
    recover.recover_step s (.JournalRead j) ⦃ p =>
      p.2 = .Abort .Malformed ∨ p.2 = .Abort .MarkerMissing ⦄ := by
  simp only [recover.recover_step,atRead,recover.on_journal]
  split
  · simp [recover.refuse]
  · rename_i ordered
    rcases bad with impossible | overflow | ⟨hm,hd⟩
    · exact False.elim (ordered impossible)
    · simp [overflow,lift,recover.refuse]
    · cases count : U32.checked_add j.entries j.directories with
      | none => simp [lift,recover.refuse]
      | some n =>
        have ne : (j.directories != 0#u32) = true := bne_iff_ne.mpr hd
        simp [lift,hm,ne,recover.refuse]

/-! A nonempty admitted legacy traversal, without any directory or committed
premise. The entry already matches its absent prior: the kernel issues a
`Seal`, and cannot retire the record until the host answers it. -/
def legacyOneJournal : recover.JournalFacts := ⟨1#u32,0#u32,0#u32,none⟩
def absentLeaf : recover.EntryFacts := ⟨⟨0#u32⟩,.Absent,none,.Removed,.Leaf⟩
def legacySealLog : Log := [
  (.ReadJournal,.JournalRead legacyOneJournal), (.ReadEntry 0#u32,.EntryRead absentLeaf),
  (.Observe 0#u32 absentLeaf.dest,.Observed absentLeaf.dest .Absent),
  (.Seal 0#u32 absentLeaf.dest .Absent .Absent,.Done)]
def legacySealState : recover.RecoverState :=
  ⟨.UnlinkingRecord,legacyOneJournal,1#u32,none,.Uncommitted,0#u32,false,none⟩

theorem legacy_seal_prefix :
    Prefix legacySealLog legacySealState (.Unlink (.Record 0#u32)) := by
  let initial : recover.RecoverState :=
    ⟨.Start,⟨0#u32,0#u32,0#u32,none⟩,0#u32,none,.Pending,0#u32,false,none⟩
  let journal := {initial with phase := .ReadingJournal}
  let entry := {legacySealState with phase := .ReadingEntry}
  let obs := {entry with phase := .Observing,entry := some absentLeaf}
  let sealing := {obs with phase := .Sealing}
  have one {a b : U32} (h : a.val + b.val = 1) : U32.checked_add a b = some 1#u32 := by
    have spec := U32.checked_add_bv_spec a b
    cases e : U32.checked_add a b with
    | none => simp [e,h,U32.max_eq] at spec
    | some n =>
      simp only [e] at spec
      exact congrArg some (UScalar.eq_of_val_eq (spec.2.1.trans h))
  have begin : Prefix [] journal .ReadJournal := .start (initial := initial) rfl rfl
  have read := begin.answer (e := .JournalRead legacyOneJournal) (t := entry)
    (d := .ReadEntry 0#u32) (by simp [IsCleanup]) trivial (by
      simp [journal,initial,recover.recover_step,recover.on_journal,legacyOneJournal,
        recover.proceed,entry,legacySealState,lift,one (a := 1#u32) (b := 0#u32) rfl])
  have observe := read.answer (e := .EntryRead absentLeaf) (t := obs)
    (d := .Observe 0#u32 absentLeaf.dest) (by simp [IsCleanup]) trivial rfl
  have issued := observe.answer (e := .Observed absentLeaf.dest .Absent) (t := sealing)
    (d := .Seal 0#u32 absentLeaf.dest .Absent .Absent) (by simp [IsCleanup]) rfl (by
      simp [obs,entry,sealing,legacySealState,recover.recover_step,recover.on_observed,
        absentLeaf,ids.DestId.same,recover.decide])
  exact issued.answer (e := .Done) (by simp [IsCleanup]) trivial (by
    simp [sealing,obs,entry,legacySealState,legacyOneJournal,recover.recover_step,
      recover.advance,recover.proceed,recover.begin_cleanup,recover.next_unlink,lift,
      one (a := 0#u32) (b := 1#u32) rfl])

end Gripsack.Protocol.RecoverWalk
