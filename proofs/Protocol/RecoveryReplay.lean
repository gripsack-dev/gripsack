import Protocol.RecoverLeaf

/-! Ring-2 interpretation of the command/answer prefix before cleanup. Ring-3
readers supply the indexed record codec and the admitted selection chain. A
logical `Done` never produces a durable leaf: the storage reply must produce it.
S4/S5 and non-aliasing are expressed as per-operation footprints, including the
inode, not as a premise that the desired post-crash observation already holds. -/
namespace Gripsack.Protocol.RecoveryReplay

open Gripsack.Storage Gripsack.Protocol.RecoverLeaf Gripsack.Protocol.Publication
open Aeneas.Std Aeneas.Std.Result gripsack_kernel

abbrev Exchange := recover.Command × recover.Event

structure Row where
  recordLoc : Loc
  leaf : Loc
  facts : recover.EntryFacts
  prior : Value
  observed : Value

structure Context where
  codec : Codec
  row : U32 → Row
  /-- Admission interprets the record bytes and its original blob (or inline
  symlink/absence), including exact original permissions. Not a persistence law. -/
  decodeEntry : Image → FileData → Option (recover.EntryFacts × Value)
  current : Loc
  pointer : String
  manifest : Loc
  manifestInode : Nat
  manifestData : FileData
  /-- Admitted selection metadata maps the pointer AND manifest to identity;
  a generation number or a symlink string alone cannot authorize cleanup. -/
  decodeCurrent : String → FileData → Option selection.SelectionIdentity
  selectionChain : List Leaf.Fact
  selectionFacts : recover.JournalFacts
  /-- The frozen host inventory, including the separate directory domain. -/
  names : List Loc
  count : names.length = selectionFacts.entries.val + selectionFacts.directories.val
  named : ∀ i : U32, i.val < selectionFacts.entries.val → names[i.val]? = some (row i).recordLoc

/-- What the host actually read; malformed/error outcomes never give this response. -/
def Read (k : Context) (m : Machine) : Exchange → Prop
  | (.ReadJournal, .JournalRead j) => j = k.selectionFacts
  | (.ReadEntry i, .EntryRead f) => f = (k.row i).facts ∧
      ((m.fs.visible.read (k.row i).recordLoc).bind (k.decodeEntry m.fs.visible)) = some (f,(k.row i).prior)
  | (.Observe i _, .Observed _ seen) => seen = (k.row i).observed.live k.codec ∧
      Holds m.kind m.fs.visible (k.row i).leaf (k.row i).observed
  | (.ReadCurrent, .CurrentRead (some cur)) => Holds m.kind m.fs.visible k.current (.link k.pointer) ∧
      (m.fs.visible.read k.manifest).bind (k.decodeCurrent k.pointer) = some cur ∧
      ∀ f ∈ k.selectionChain, f.Holds m.fs.visible
  | (.ReadCurrent, .CurrentRead none) => m.fs.visible.link k.current = none
  | _ => True

/-- The destination/final value whose persistence this particular answer earned. -/
def claim (k : Context) : Exchange → Option (Loc × Value)
  | (.Barrier .Selection, .Done) => some (k.current,.link k.pointer)
  | (.Mutate i .., .Landed _) | (.Seal i .., .Done) => some ((k.row i).leaf,(k.row i).prior)
  | (.Preserve i _, .Done) => some ((k.row i).leaf,(k.row i).observed)
  | _ => none

/-- These are successful host operations, not pre-proved consequences. A
selection fsync uses the previously admitted pointer under serialization, here
witnessed by its truthful nonmutating read at the barrier. -/
def Effect (k : Context) (m t : Machine) (p : Exchange) : Prop :=
  match p with
  | (.Barrier .Selection, .Done) => Completed m (.readLink k.current (targetBytes k.pointer)) m ∧
      Completed m (.syncDir k.current.parent) t
  | (.Mutate i .., .Landed _) | (.Seal i .., .Done) | (.Preserve i _, .Done) =>
      Reply k.codec i (k.row i).facts (k.row i).leaf (k.row i).prior (k.row i).observed p.1 p.2 m t
  | _ => True

private theorem claimed {k m t p l v} (exec : Effect k m t p) (has : claim k p = some (l,v)) :
    Durable t l v := by
  obtain ⟨cmd,ev⟩ := p
  cases cmd <;> cases ev <;> simp only [claim, reduceCtorEq, Option.some.injEq, Prod.mk.injEq] at has
  all_goals try {obtain ⟨rfl,rfl⟩ := has; exact exec.durable}
  case Barrier.Done scope =>
    cases scope <;> simp at has
    obtain ⟨rfl,rfl⟩ := has
    exact (Seal.link exec.1 exec.2).durable

/-- Per admitted operation. `private` coordinates include durable journal
bytes and any source/selection manifest ancestors relevant to this invocation.
A directory callback is covered only to the extent its (D) footprint is disjoint.
Normal forms for successful leaf commands are provided by `Reply.normalForm`. -/
structure Answer (k : Context) (reserved : Leaf.Fact → Prop) (history : List Exchange)
    (p : Exchange) (m t : Machine) where
  read : Read k m p
  effect : Effect k m t p
  ops : List Leaf.Op
  disk : Simulates id t.fs (Leaf.execOps m.fs ops)
  kinds : KindsExtend m t
  privateNames : ∀ op ∈ ops, ∀ f, reserved f → op.Away f
  otherLeaves : ∀ z ∈ history, ∀ l v, claim k z = some (l,v) → ∀ op ∈ ops, Away m l op

inductive Replay (k : Context) (reserved : Leaf.Fact → Prop) (base : Machine) :
    List Exchange → Machine → Prop where
  | start : Replay k reserved base [] base
  | snoc {h m p t} : Replay k reserved base h m → Answer k reserved h p m t →
      Replay k reserved base (h ++ [p]) t

theorem Replay.keeps {k reserved base h m} (r : Replay k reserved base h m) {f : Leaf.Fact}
    (p : reserved f) (s : f.Stable base.fs) : f.Stable m.fs := by
  induction r with
  | start => exact s
  | snoc prev a ih => exact sim_all a.disk (Leaf.exec_frame ih (fun op ho => a.privateNames op ho f p))

theorem Replay.claimed {k reserved base h m} (r : Replay k reserved base h m) {p l v}
    (mem : p ∈ h) (has : claim k p = some (l,v)) : Durable m l v := by
  induction r with
  | start => simp at mem
  | snoc prev a ih =>
    rcases List.mem_append.mp mem with earlier | last
    · exact (ih earlier).frame_refined a.disk a.kinds (a.otherLeaves p earlier l v has)
    · rcases List.mem_singleton.mp last with rfl
      exact RecoveryReplay.claimed a.effect has

theorem Replay.at_answer {k reserved base h m} (r : Replay k reserved base h m) {p} (mem : p ∈ h) :
    ∃ h₀ s t, Replay k reserved base h₀ s ∧ Read k s p ∧ Effect k s t p := by
  induction r with
  | start => simp at mem
  | snoc prev a ih =>
    rcases List.mem_append.mp mem with before | last
    · exact ih before
    · cases List.mem_singleton.mp last
      exact ⟨_,_,_,prev,a.read,a.effect⟩

theorem Replay.answered {k reserved base h m} (r : Replay k reserved base h m) {p} (mem : p ∈ h) :
    ∃ s t, Read k s p ∧ Effect k s t p := by
  obtain ⟨_,s,t,_,read,exec⟩ := r.at_answer mem
  exact ⟨s,t,read,exec⟩

/-- Before cleanup, even an interrupted staging operation can only disturb its
own coordinates. In particular, a failed read/restore does not consume the
last readable P1 packet. This permits arbitrary prefixes (and partial writes)
on private staging: no success, completed barrier or rollback is assumed. -/
theorem failure_keeps_evidence {k reserved base h m} (r : Replay k reserved base h m)
    {f : Leaf.Fact} (keep : reserved f) (stable : f.Stable base.fs) {ops t}
    (stopped : Simulates id t (Leaf.execOps m.fs ops)) (away : ∀ op ∈ ops, op.Away f) : f.Stable t :=
  sim_all stopped (Leaf.exec_frame (r.keeps keep stable) away)

end Gripsack.Protocol.RecoveryReplay
