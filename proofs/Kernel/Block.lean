import Extracted.GripsackKernel
import Kernel.Recover
import Kernel.Retention

/-!
# Managed blocks (plan/0056 §5 K5, K1 for blocks)

Theorems over the Aeneas translation of `gripsack_kernel::block`
(`Extracted/GripsackKernel.lean`).

* `splice_spec`: the splice output is the text before the first span, the
  replacement, then every byte between and after the spans, verbatim and in
  order (`spliceFrom`, a plain recursion over the spans).
* `upsert_preserves_outside` (K5): an upsert that returns `out` scanned
  ordered, disjoint, in-bounds blocks and returned exactly the splice of the
  rendered block into them — or, with no block, the text, one separating
  newline if it lacked one, then the rendered block.
* `remove_preserves_outside` (K5): a removal keeps exactly the bytes outside
  the scanned blocks.
* `upsert_idempotent` (K5): upserting an upsert's output again with the same
  inputs returns it unchanged.
* `upsert_round_trip`: the output holds exactly one block of the owner, which
  is the rendered block, whose marker records the given digest's sha digits
  and mode, and whose content normalizes to the payload's normalization.
* `decide_block_remove_intact` (K1 for blocks): a block is removed only if it
  is intact.
* `decide_block_foreign_untouched`, `decide_block_mode_evidence_preserves`,
  `decide_block_restore_exact`: a non-file host is never written; conflicting
  mode evidence never writes; rollback writes (or claims) a block only from a
  retained payload whose digest is the record.

The K5 theorems are partial-correctness statements: "if the call returns
`out`, then …". Upsert returns a refusal rather than an output that does not
read back as written; that it never refuses a well-formed text is tested by
the kernel explorer (`crates/gripsack-kernel/tests/block.rs`), not proved.
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel.Block

/-! ## Partial correctness over `Result` -/

theorem bind_eq_ok {α β : Type} {x : Result α} {f : α → Result β} {y : β} :
    Std.bind x f = ok y ↔ ∃ a, x = ok a ∧ f a = ok y := by
  constructor
  · intro h
    cases x with
    | ret a => exact ⟨a, rfl, by simpa [bind_ok] using h⟩
    | vis e k =>
      rw [bind_vis] at h
      exact absurd h vis_not_ok
    | div =>
      rw [bind_div] at h
      exact absurd h div_not_ok
  · rintro ⟨a, rfl, h⟩
    simpa [bind_ok] using h

theorem bind_tc_eq_ok {α β : Type} {x : Result α} {f : α → Result β} {y : β} :
    (x >>= f) = ok y ↔ ∃ a, x = ok a ∧ f a = ok y := bind_eq_ok

/-- A loop that returns satisfies the postcondition its body preserves. -/
theorem loop_ok {α β : Type} (measure : α → Nat) (inv : α → Prop) (post : β → Prop)
    (body : α → Result (ControlFlow α β))
    (hbody : ∀ x r, inv x → body x = ok r →
      match r with
      | .done y => post y
      | .cont x' => inv x' ∧ measure x' < measure x) :
    ∀ x y, inv x → loop body x = ok y → post y := by
  intro x
  generalize hn : measure x = n
  induction n using Nat.strong_induction_on generalizing x with
  | _ n ih =>
    intro y hx hl
    unfold loop at hl
    rw [bind_eq_ok] at hl
    obtain ⟨r, hr, hl⟩ := hl
    have hb := hbody x r hx hr
    cases r with
    | done z =>
      simp only [Result.ok.injEq] at hl
      subst hl
      exact hb
    | cont x' =>
      simp only at hl
      exact ih (measure x') (hn ▸ hb.2) x' rfl y hb.1 hl

/-! ## What the primitive steps return -/

theorem index_usize_ok {α : Type} {s : Slice α} {i : Usize} {x : α}
    (h : Slice.index_usize s i = ok x) : s.val[i.val]? = some x := by
  unfold Slice.index_usize at h
  split at h
  · exact absurd h fail_not_ok
  · rename_i y hy
    simp only [Result.ok.injEq] at h
    subst h
    simpa using hy

theorem range_ok {s s1 : Slice U8} {r : core.ops.range.Range Usize}
    (h : core.slice.index.Slice.index (core.slice.index.SliceIndexRangeUsizeSlice U8) s r = ok s1) :
    r.start.val ≤ r.end.val ∧ r.end.val ≤ s.length ∧ s1.val = s.val.slice r.start.val r.end.val := by
  rw [Slice.index_SliceIndexRangeUsizeSliceInst] at h
  unfold core.slice.index.SliceIndexRangeUsizeSlice.index at h
  split_ifs at h with hb
  · simp only [Result.ok.injEq] at h
    subst h
    refine ⟨?_, ?_, ?_⟩
    · have := hb.1; scalar_tac
    · have := hb.2; scalar_tac
    · simp
  · exact absurd h fail_not_ok

theorem range_from_ok {s s1 : Slice U8} {r : core.ops.range.RangeFrom Usize}
    (h : core.slice.index.Slice.index (core.slice.index.SliceIndexRangeFromUsizeSlice U8) s r = ok s1) :
    s1.val = s.val.drop r.start.val := by
  rw [Slice.index_SliceIndexRangeFromUsizeSliceInst] at h
  unfold core.slice.index.SliceIndexRangeFromUsizeSlice.index at h
  split_ifs at h
  · simp only [Result.ok.injEq] at h
    subst h
    simp [Slice.drop]
  · exact absurd h fail_not_ok

theorem extend_ok {v r : alloc.vec.Vec U8} {s : Slice U8}
    (h : alloc.vec.Vec.extend_from_slice core.clone.CloneU8 v s = ok r) :
    r.val = v.val ++ s.val := by
  unfold alloc.vec.Vec.extend_from_slice at h
  split_ifs at h
  · rw [bind_tc_eq_ok] at h
    obtain ⟨s', hs', h⟩ := h
    simp only [Result.ok.injEq] at h
    subst h
    have := List.clone_spec (clone := core.clone.CloneU8.clone) (l := s.val) (by simp)
    obtain ⟨c, hc, hp⟩ := spec_exists_ok this
    rw [hs'] at hc
    simp only [Result.ok.injEq] at hc
    subst hc
    simp [alloc.vec.Vec.from, alloc.vec.Vec.val, hp.1]
  · exact absurd h fail_not_ok

theorem sub_ok {x y z : Usize} (h : x - y = ok z) : z.val = x.val - y.val ∧ y.val ≤ x.val := by
  have := UScalar.sub_equiv x y
  rw [h] at this
  simp only [Result.match.ok] at this
  omega

@[simp] theorem new_val : (alloc.vec.Vec.new U8).val = [] := rfl

theorem add_ok {x y z : Usize} (h : x + y = ok z) : z.val = x.val + y.val := by
  have := UScalar.add_equiv x y
  rw [h] at this
  simp only [Result.match.ok] at this
  exact this.2.1

/-! ## The splice -/

/-- The splice as a recursion over the spans: the bytes from `cursor` up to the
next span, the replacement at the first span only, then the rest; after the
last span, every remaining byte. -/
def spliceFrom (text repl : List U8) : List block.Block → Nat → Bool → List U8
  | [], cursor, _ => text.drop cursor
  | b :: bs, cursor, first =>
    text.slice cursor b.start.val ++ (if first then repl else []) ++
      spliceFrom text repl bs b.end.val false

/-- Replace the first span with `repl`, drop the others, keep every other byte. -/
def spliceSpec (text repl : List U8) (spans : List block.Block) : List U8 :=
  spliceFrom text repl spans 0 true

theorem splice_loop_ok (text repl : Slice U8) (blocks : Slice block.Block)
    (out : alloc.vec.Vec U8) (cursor i : Usize) (res : alloc.vec.Vec U8 × Usize)
    (hinv : out.val ++ spliceFrom text.val repl.val (blocks.val.drop i.val) cursor.val
        (decide (i.val = 0)) = spliceSpec text.val repl.val blocks.val)
    (h : block.splice_loop text blocks repl out cursor i = ok res) :
    res.1.val ++ text.val.drop res.2.val = spliceSpec text.val repl.val blocks.val := by
  unfold block.splice_loop at h
  refine loop_ok (fun s : alloc.vec.Vec U8 × Usize × Usize => blocks.val.length - s.2.2.val)
    (fun s => s.1.val ++ spliceFrom text.val repl.val (blocks.val.drop s.2.2.val) s.2.1.val
        (decide (s.2.2.val = 0)) = spliceSpec text.val repl.val blocks.val)
    (fun r : alloc.vec.Vec U8 × Usize =>
      r.1.val ++ text.val.drop r.2.val = spliceSpec text.val repl.val blocks.val)
    _ ?_ (out, cursor, i) res hinv h
  rintro ⟨out, cursor, i⟩ r hinv hr
  dsimp only at hinv hr
  unfold block.splice_loop.body at hr
  dsimp only at hr
  by_cases hlt : i < blocks.len
  · simp only [hlt, ↓reduceIte, bind_eq_ok] at hr
    obtain ⟨b, hb, sl, hsl, out1, hout1, out2, hout2, i2, hi2, hr⟩ := hr
    simp only [Result.ok.injEq] at hr
    subst hr
    have hb' := index_usize_ok hb
    have hlt' : i.val < blocks.val.length := by scalar_tac
    have hsl' := (range_ok hsl).2.2
    have hout1' := extend_ok hout1
    have hi2' := add_ok hi2
    have hdrop : blocks.val.drop i.val = b :: blocks.val.drop (i.val + 1) := by
      rw [List.drop_eq_getElem_cons hlt']
      congr 1
      simpa [List.getElem?_eq_getElem hlt'] using hb'
    simp only
    refine ⟨?_, by scalar_tac⟩
    rw [← hinv, hdrop, hi2']
    simp only [spliceFrom]
    split_ifs at hout2 with hz
    · have := extend_ok hout2
      rw [this, hout1', hsl']
      have : decide (i.val = 0) = true := by simp; scalar_tac
      simp [this]
    · simp only [Result.ok.injEq] at hout2
      subst hout2
      rw [hout1', hsl']
      have : decide (i.val = 0) = false := by simp; scalar_tac
      simp [this]
  · simp only [hlt, ↓reduceIte] at hr
    simp only [Result.ok.injEq] at hr
    subst hr
    simp only
    have hge : blocks.val.length ≤ i.val := by scalar_tac
    rw [← hinv, List.drop_eq_nil_of_le hge]
    simp [spliceFrom]

/-- The splice returns exactly `spliceSpec` (whenever it returns). -/
theorem splice_ok (text repl : Slice U8) (blocks : Slice block.Block) (out : alloc.vec.Vec U8)
    (h : block.splice text blocks repl = ok out) :
    out.val = spliceSpec text.val repl.val blocks.val := by
  unfold block.splice at h
  rw [bind_eq_ok] at h
  obtain ⟨res, hres, h⟩ := h
  obtain ⟨acc, cursor⟩ := res
  simp only [Std.uncurry] at h
  rw [bind_eq_ok] at h
  obtain ⟨tail, htail, h⟩ := h
  have hl := splice_loop_ok text repl blocks _ _ _ _ (by simp [spliceSpec]) hres
  rw [extend_ok h, range_from_ok htail]
  exact hl


theorem push_ok {v r : alloc.vec.Vec U8} {x : U8} (h : alloc.vec.Vec.push v x = ok r) :
    r.val = v.val ++ [x] := by
  unfold alloc.vec.Vec.push at h
  dsimp only at h
  split_ifs at h
  · simp only [Result.ok.injEq] at h
    subst h
    simp [alloc.vec.Vec.from, alloc.vec.Vec.val]
  · exact absurd h fail_not_ok

/-- The bytes of a newline convention. -/
def newlineBytes : block.Newline → List U8
  | .Lf => [10#u8]
  | .CrLf => [13#u8, 10#u8]

theorem push_newline_ok {v r : alloc.vec.Vec U8} {n : block.Newline}
    (h : block.push_newline v n = ok r) : r.val = v.val ++ newlineBytes n := by
  unfold block.push_newline at h
  rw [bind_eq_ok] at h
  obtain ⟨v1, hv1, h⟩ := h
  rw [push_ok h]
  cases n with
  | Lf =>
    simp only [Result.ok.injEq] at hv1
    subst hv1
    simp [newlineBytes]
  | CrLf =>
    simp only at hv1
    rw [push_ok hv1]
    simp [newlineBytes]

/-! ## Appending to a text without a block -/

/-- The newline appended before a block when the text does not end in one. -/
def separator (text : List U8) (n : block.Newline) : List U8 :=
  if text ≠ [] ∧ text.getLast? ≠ some 10#u8 then newlineBytes n else []

theorem append_ok (text rendered : Slice U8) (n : block.Newline) (out : alloc.vec.Vec U8)
    (h : block.append text n rendered = ok out) :
    out.val = text.val ++ separator text.val n ++ rendered.val := by
  unfold block.append at h
  rw [bind_eq_ok] at h
  obtain ⟨v, hv, h⟩ := h
  rw [bind_eq_ok] at h
  obtain ⟨v1, hv1, h⟩ := h
  rw [extend_ok h]
  have hv' := extend_ok hv
  rw [new_val, List.nil_append] at hv'
  split_ifs at hv1 with hpos
  · rw [bind_eq_ok] at hv1
    obtain ⟨last, hlast, hv1⟩ := hv1
    rw [bind_eq_ok] at hv1
    obtain ⟨x, hx, hv1⟩ := hv1
    have hl := sub_ok hlast
    have hx' := index_usize_ok hx
    have hne : text.val ≠ [] := by
      intro he; have : text.len.val = 0 := by simp [he]
      scalar_tac
    have hlast' : text.val.getLast? = some x := by
      rw [List.getLast?_eq_getElem?, ← hx']
      congr 1
      have : text.len.val = text.val.length := by simp
      have h1 : (1#usize).val = 1 := rfl
      omega
    split_ifs at hv1 with hnl
    · rw [push_newline_ok hv1, hv']
      have hx10 : x ≠ 10#u8 := by simpa [bne_iff_ne] using hnl
      have : separator text.val n = newlineBytes n := by
        simp [separator, hne, hlast', hx10]
      rw [this]
    · simp only [Result.ok.injEq] at hv1
      subst hv1
      rw [hv']
      have hx10 : x = 10#u8 := by
        simp only [bne_iff_ne, ne_eq, Decidable.not_not] at hnl
        exact hnl
      have : separator text.val n = [] := by
        simp [separator, hlast', hx10]
      rw [this, List.append_nil]
  · simp only [Result.ok.injEq] at hv1
    subst hv1
    rw [hv']
    have hnil : text.val = [] := by
      have : text.len.val = 0 := by scalar_tac
      simpa using this
    simp [separator, hnil]


/-! ## The checks upsert relies on -/

/-- Spans in order, disjoint, non-empty and inside `len` bytes, from `cursor`. -/
def OrderedFrom (len : Nat) : List block.Block → Nat → Prop
  | [], _ => True
  | b :: bs, cursor =>
    cursor ≤ b.start.val ∧ b.start.val < b.end.val ∧ b.end.val ≤ len ∧ OrderedFrom len bs b.end.val

/-- The scanned blocks are ordered, disjoint, non-empty and in bounds. -/
def Ordered (spans : List block.Block) (len : Nat) : Prop := OrderedFrom len spans 0

theorem ordered_ok (blocks : Slice block.Block) (len : Usize)
    (h : block.ordered blocks len = ok true) : Ordered blocks.val len.val := by
  unfold block.ordered block.ordered_loop at h
  refine loop_ok (fun s : Usize × Usize => blocks.val.length - s.2.val)
    (fun s => OrderedFrom len.val (blocks.val.drop s.2.val) s.1.val → Ordered blocks.val len.val)
    (fun r : Bool => r = true → Ordered blocks.val len.val)
    _ ?_ (0#usize, 0#usize) true (by simp [Ordered]) h rfl
  rintro ⟨cursor, i⟩ r hinv hr
  dsimp only at hinv hr
  unfold block.ordered_loop.body at hr
  dsimp only at hr
  by_cases hlt : i < blocks.len
  · simp only [hlt, ↓reduceIte, bind_eq_ok] at hr
    obtain ⟨b, hb, hr⟩ := hr
    have hb' := index_usize_ok hb
    have hlt' : i.val < blocks.val.length := by scalar_tac
    split_ifs at hr with h1 h2 h3
    · simp only [Result.ok.injEq] at hr; subst hr; simp
    · simp only [Result.ok.injEq] at hr; subst hr; simp
    · simp only [Result.ok.injEq] at hr; subst hr; simp
    · rw [bind_eq_ok] at hr
      obtain ⟨i2, hi2, hr⟩ := hr
      simp only [Result.ok.injEq] at hr
      subst hr
      have hi2' := add_ok hi2
      refine ⟨fun hrest => hinv ?_, by simp only; scalar_tac⟩
      have hdrop : blocks.val.drop i.val = b :: blocks.val.drop (i.val + 1) := by
        rw [List.drop_eq_getElem_cons hlt']
        congr 1
        simpa [List.getElem?_eq_getElem hlt'] using hb'
      rw [hdrop]
      simp only at hrest
      rw [hi2'] at hrest
      exact ⟨by scalar_tac, by scalar_tac, by scalar_tac, hrest⟩
  · simp only [hlt, ↓reduceIte, Result.ok.injEq] at hr
    subst hr
    intro _
    apply hinv
    rw [List.drop_eq_nil_of_le (by scalar_tac)]
    simp [OrderedFrom]

theorem matches_at_ok (text key : Slice U8) (at' : Usize)
    (h : block.matches_at text at' key = ok true) :
    ∀ j, j < key.length → text.val[at'.val + j]? = key.val[j]? := by
  unfold block.matches_at block.matches_at_loop at h
  refine loop_ok (fun j : Usize => key.length - j.val)
    (fun j : Usize => ∀ j', j' < j.val → text.val[at'.val + j']? = key.val[j']?)
    (fun r : Bool => r = true → ∀ j, j < key.length → text.val[at'.val + j]? = key.val[j]?)
    _ ?_ 0#usize true (by simp) h rfl
  intro j r hinv hr
  unfold block.matches_at_loop.body at hr
  dsimp only at hr
  by_cases hlt : j < key.len
  · simp only [hlt, ↓reduceIte, bind_eq_ok] at hr
    obtain ⟨k, hk, x, hx, y, hy, hr⟩ := hr
    have hk' := add_ok hk
    have hx' := index_usize_ok hx
    have hy' := index_usize_ok hy
    split_ifs at hr with hne
    · simp only [Result.ok.injEq] at hr; subst hr; simp
    · rw [bind_eq_ok] at hr
      obtain ⟨j1, hj1, hr⟩ := hr
      simp only [Result.ok.injEq] at hr
      subst hr
      have hj1' := add_ok hj1
      simp only [bne_iff_ne, ne_eq, Decidable.not_not] at hne
      refine ⟨fun j' hj' => ?_, by scalar_tac⟩
      by_cases hjj : j' < j.val
      · exact hinv j' hjj
      · have : j' = j.val := by scalar_tac
        subst this
        rw [← hk', hx', hy', hne]
  · simp only [hlt, ↓reduceIte, Result.ok.injEq] at hr
    subst hr
    intro _ j' hj'
    exact hinv j' (by scalar_tac)

theorem holds_ok (text bytes : Slice U8) (start end_ : Usize)
    (h : block.holds text start end_ bytes = ok true) :
    start.val ≤ end_.val ∧ end_.val ≤ text.length ∧
      text.val.slice start.val end_.val = bytes.val := by
  unfold block.holds at h
  dsimp only at h
  by_cases h1 : end_ ≤ text.len
  · by_cases h2 : start ≤ end_
    · simp only [h1, h2, ↓reduceIte, bind_eq_ok] at h
      obtain ⟨d, hd, h⟩ := h
      have hd' := sub_ok hd
      by_cases h3 : d = bytes.len
      · simp only [h3, ↓reduceIte] at h
        have hm := matches_at_ok text bytes start h
        have hdv : d.val = bytes.length := by rw [h3]; simp
        refine ⟨by scalar_tac, by scalar_tac, ?_⟩
        apply List.ext_getElem?
        intro j
        by_cases hj : j < bytes.length
        · rw [← hm j hj]
          simp only [List.slice, List.getElem?_take, List.getElem?_drop]
          have : j < end_.val - start.val := by omega
          simp [this, Nat.add_comm]
        · have hlen : (text.val.slice start.val end_.val).length = bytes.val.length := by
            simp only [List.slice, List.length_take, List.length_drop]
            have : end_.val ≤ text.val.length := by scalar_tac
            have : bytes.length = bytes.val.length := rfl
            omega
          rw [List.getElem?_eq_none (by rw [hlen]; exact Nat.le_of_not_lt hj),
            List.getElem?_eq_none (by exact Nat.le_of_not_lt hj)]
      · simp only [h3, ↓reduceIte] at h
        exact absurd h (by simp)
    · simp only [h1, h2, ↓reduceIte] at h
      exact absurd h (by simp)
  · simp only [h1, ↓reduceIte] at h
    exact absurd h (by simp)

theorem newline_same_ok {a b : block.Newline} (h : block.Newline.same a b = ok true) : a = b := by
  cases a <;> cases b <;> simp_all [block.Newline.same]

theorem same_mode_ok {a : Option U32} {m : U32} (h : block.same_mode a m = ok true) :
    a = some m := by
  cases a <;> simp_all [block.same_mode]


theorem ok_false_ne_true : (ok false : Result Bool) ≠ ok true := by simp

/-- A vector whose bytes `matches_at` from 0 against an equally long slice. -/
theorem matches_whole {v : alloc.vec.Vec U8} {s : Slice U8}
    (hlen : alloc.vec.Vec.len v = Slice.len s)
    (h : block.matches_at (alloc.vec.Vec.deref v) 0#usize s = ok true) : v.val = s.val := by
  have hm := matches_at_ok _ _ _ h
  have hl : v.val.length = s.val.length := by
    have := congrArg UScalar.val hlen; simpa using this
  apply List.ext_getElem?
  intro j
  by_cases hj : j < s.val.length
  · have := hm j (by simpa using hj)
    simpa [deref_val] using this
  · rw [List.getElem?_eq_none (by omega), List.getElem?_eq_none (by omega)]

theorem ite_ok_true {c : Prop} [Decidable c] {x : Result Bool} :
    (if c then x else ok false) = ok true ↔ c ∧ x = ok true := by
  by_cases hc : c <;> simp [hc]

theorem ite_false_ok_true {c : Prop} [Decidable c] {x : Result Bool} :
    (if c then ok false else x) = ok true ↔ ¬ c ∧ x = ok true := by
  by_cases hc : c <;> simp [hc]

/-- What `settled` established when it accepted an output. -/
theorem settled_ok (out owner rendered normalized : Slice U8) (n : block.Newline)
    (sha : Array U8 16#usize) (mode : U32)
    (h : block.settled out owner rendered normalized n sha mode = ok true) :
    ∃ blocks b, block.scan out owner = ok (.Ok blocks) ∧ blocks.val = [b] ∧
      block.ordered (alloc.vec.Vec.deref blocks) (Slice.len out) = ok true ∧
      b.start.val ≤ b.end.val ∧ b.end.val ≤ out.length ∧
      out.val.slice b.start.val b.end.val = rendered.val ∧
      block.newline_of out = ok n ∧
      out.val.slice b.sha.val (b.sha.val + 16) = sha.val ∧
      b.mode = some mode ∧
      ∃ (content : Slice U8) (normal : alloc.vec.Vec U8),
        content.val = out.val.slice b.content_start.val b.content_end.val ∧
        block.normalize content = ok normal ∧ normal.val = normalized.val := by
  unfold block.settled at h
  rw [bind_eq_ok] at h
  obtain ⟨r, hscan, h⟩ := h
  cases r with
  | Err e => exact absurd h ok_false_ne_true
  | Ok blocks =>
    dsimp only at h
    simp only [bind_eq_ok, ite_ok_true, ite_false_ok_true] at h
    obtain ⟨h1, o, ho, h2, found, hfound, b1, hb1, h3, n', hn', b2, hb2, h4, i2, hi2, s1, hs1,
      b3, hb3, h5, b4, hb4, h6, h7, h8, s2, hs2, content, hcontent, h9, h10⟩ := h
    subst h2 h3 h4 h5 h6
    have hlen1 : blocks.val.length = 1 := by
      simp only [bne_iff_ne, ne_eq, Decidable.not_not] at h1
      have := congrArg UScalar.val h1; simpa using this
    obtain ⟨b, hb⟩ : ∃ b, blocks.val = [b] := List.length_eq_one_iff.mp hlen1
    rw [alloc.vec.Vec.index_slice_index] at hfound
    have hfound' : found = b := by
      unfold alloc.vec.Vec.index_usize at hfound
      split at hfound
      · exact absurd hfound fail_not_ok
      · rename_i x hx
        simp only [Result.ok.injEq] at hfound
        subst hfound
        rw [alloc.vec.Vec.getElem?_Nat_eq] at hx
        simp [hb] at hx
        exact hx.symm
    subst hfound'
    obtain ⟨hs, he, hr⟩ := holds_ok _ _ _ _ hb1
    have hi2' := add_ok hi2
    simp only [lift, Result.ok.injEq] at hs1
    subst hs1
    obtain ⟨_, _, hsha⟩ := holds_ok _ _ _ _ hb3
    have hsha16 : i2.val = found.sha.val + 16 := by
      rw [hi2']; simp [block.MARK_DIGITS]
    rw [hsha16] at hsha
    obtain ⟨_, _, hs2'⟩ := range_ok hs2
    refine ⟨blocks, found, hscan, hb, ho, hs, he, hr, ?_, ?_, same_mode_ok hb4, s2, content,
      hs2', hcontent, matches_whole h9 h10⟩
    · rw [hn', newline_same_ok hb2]
    · rw [hsha]; simp [Array.to_slice]


/-! ## Upsert and remove -/

theorem ite_eq_ok {α : Type} {c : Prop} [Decidable c] {x y : Result α} {z : α} :
    (if c then x else y) = ok z ↔ (c ∧ x = ok z) ∨ (¬ c ∧ y = ok z) := by
  by_cases hc : c <;> simp [hc]

/-- What an upsert that returned `out` computed on the way. -/
theorem upsert_ok (text owner : Slice U8) (style : block.Style) (digest : ids.Digest)
    (mode : U32) (payload : Slice U8) (out : alloc.vec.Vec U8)
    (h : block.upsert text owner style digest mode payload = ok (.Ok out)) :
    ∃ blocks newline normalized sha rendered,
      block.scan text owner = ok (.Ok blocks) ∧
      block.ordered (alloc.vec.Vec.deref blocks) (Slice.len text) = ok true ∧
      block.newline_of text = ok newline ∧
      block.normalize payload = ok normalized ∧
      block.mark digest = ok sha ∧
      block.render style owner sha mode newline (alloc.vec.Vec.deref normalized) = ok rendered ∧
      ((blocks.val = [] ∧ block.append text newline (alloc.vec.Vec.deref rendered) = ok out) ∨
        (blocks.val ≠ [] ∧
          block.splice text (alloc.vec.Vec.deref blocks) (alloc.vec.Vec.deref rendered) = ok out)) ∧
      block.settled (alloc.vec.Vec.deref out) owner (alloc.vec.Vec.deref rendered)
        (alloc.vec.Vec.deref normalized) newline sha mode = ok true := by
  unfold block.upsert at h
  rw [bind_eq_ok] at h
  obtain ⟨r, hscan, h⟩ := h
  cases r with
  | Err e => simp at h
  | Ok blocks =>
    dsimp only at h
    simp only [bind_eq_ok, ite_eq_ok] at h
    obtain ⟨o, ho, h⟩ := h
    rcases h with ⟨ho', h⟩ | ⟨_, h⟩
    swap
    · simp at h
    subst ho'
    obtain ⟨newline, hnl, normalized, hnorm, sha, hsha, rendered, hrender, out', hout, b1, hb1, h⟩ := h
    rcases h with ⟨hb1', h⟩ | ⟨_, h⟩
    swap
    · simp at h
    subst hb1'
    simp only [Result.ok.injEq, core.result.Result.Ok.injEq] at h
    subst h
    refine ⟨blocks, newline, normalized, sha, rendered, hscan, ho, hnl, hnorm, hsha, hrender, ?_, hb1⟩
    rcases hout with ⟨hz, hout⟩ | ⟨hz, hout⟩
    · left
      refine ⟨?_, hout⟩
      have := congrArg UScalar.val hz
      simpa using this
    · right
      refine ⟨?_, hout⟩
      intro hnil
      apply hz
      apply UScalar.eq_of_val_eq
      simp [hnil]

/-- K5, upsert: the output is the splice of the rendered block into the
scanned blocks — the bytes before the first block, the block, then every byte
between and after the owner's blocks, verbatim and in order — or, with no
block, the text, one separating newline if it lacked one, then the block. The
scanned blocks are ordered, disjoint, non-empty and in bounds. -/
theorem upsert_preserves_outside (text owner : Slice U8) (style : block.Style)
    (digest : ids.Digest) (mode : U32) (payload : Slice U8) (out : alloc.vec.Vec U8)
    (h : block.upsert text owner style digest mode payload = ok (.Ok out)) :
    ∃ blocks newline normalized sha rendered,
      block.scan text owner = ok (.Ok blocks) ∧ Ordered blocks.val text.length ∧
      block.newline_of text = ok newline ∧
      block.normalize payload = ok normalized ∧ block.mark digest = ok sha ∧
      block.render style owner sha mode newline (alloc.vec.Vec.deref normalized) = ok rendered ∧
      (blocks.val = [] → out.val = text.val ++ separator text.val newline ++ rendered.val) ∧
      (blocks.val ≠ [] → out.val = spliceSpec text.val rendered.val blocks.val) := by
  obtain ⟨blocks, newline, normalized, sha, rendered, hscan, ho, hnl, hnorm, hsha, hrender,
    hout, _⟩ := upsert_ok text owner style digest mode payload out h
  have hord := ordered_ok _ _ ho
  simp only [deref_val] at hord
  refine ⟨blocks, newline, normalized, sha, rendered, hscan, by simpa using hord, hnl, hnorm, hsha,
    hrender, ?_, ?_⟩
  · intro hnil
    rcases hout with ⟨_, hout⟩ | ⟨hne, _⟩
    · rw [append_ok _ _ _ _ hout]; simp
    · exact absurd hnil hne
  · intro hne
    rcases hout with ⟨hnil, _⟩ | ⟨_, hout⟩
    · exact absurd hnil hne
    · rw [splice_ok _ _ _ _ hout]; simp

/-- K5, remove: the output keeps exactly the bytes outside the scanned blocks,
verbatim and in order. -/
theorem remove_preserves_outside (text owner : Slice U8) (out : alloc.vec.Vec U8)
    (h : block.remove text owner = ok (.Ok (some out))) :
    ∃ blocks, block.scan text owner = ok (.Ok blocks) ∧ Ordered blocks.val text.length ∧
      blocks.val ≠ [] ∧ out.val = spliceSpec text.val [] blocks.val := by
  unfold block.remove at h
  rw [bind_eq_ok] at h
  obtain ⟨r, hscan, h⟩ := h
  cases r with
  | Err e => simp at h
  | Ok blocks =>
    dsimp only at h
    simp only [bind_eq_ok, ite_eq_ok] at h
    obtain ⟨o, ho, h⟩ := h
    rcases h with ⟨ho', h⟩ | ⟨_, h⟩
    swap
    · simp at h
    subst ho'
    rcases h with ⟨_, h⟩ | ⟨hz, h⟩
    · simp at h
    obtain ⟨s2, hs2, v, hv, h⟩ := h
    simp only [Result.ok.injEq, core.result.Result.Ok.injEq, Option.some.injEq] at h
    subst h
    simp only [lift, Result.ok.injEq] at hs2
    subst hs2
    have hord := ordered_ok _ _ ho
    refine ⟨blocks, hscan, by simpa using hord, ?_, ?_⟩
    · intro hnil
      apply hz
      apply UScalar.eq_of_val_eq
      simp [hnil]
    · rw [splice_ok _ _ _ _ hv]
      simp [Array.to_slice, Std.Array.empty]


/-! ## Idempotence -/

theorem extend_spec (v : alloc.vec.Vec U8) (s : Slice U8) (h : v.length + s.length ≤ Usize.max) :
    alloc.vec.Vec.extend_from_slice core.clone.CloneU8 v s ⦃ r => r.val = v.val ++ s.val ⦄ := by
  have : ∃ r, alloc.vec.Vec.extend_from_slice core.clone.CloneU8 v s = ok r := by
    unfold alloc.vec.Vec.extend_from_slice
    split
    swap
    · contradiction
    have hc := List.clone_spec (clone := core.clone.CloneU8.clone) (l := s.val) (by simp)
    obtain ⟨c, hc, _⟩ := spec_exists_ok hc
    rw [hc]
    simp only [bind_tc_ok]
    exact ⟨_, rfl⟩
  obtain ⟨r, hr⟩ := this
  rw [hr]
  simpa using extend_ok hr

/-- Splicing a text's only span with the bytes it already holds returns the
text. -/
theorem splice_single (text repl : Slice U8) (blocks : Slice block.Block) (b : block.Block)
    (hb : blocks.val = [b]) (hs : b.start.val ≤ b.end.val) (he : b.end.val ≤ text.length)
    (hr : text.val.slice b.start.val b.end.val = repl.val) :
    block.splice text blocks repl ⦃ v => v.val = text.val ⦄ := by
  have hmax : text.length ≤ Usize.max := text.property
  have hlen1 : blocks.len.val = 1 := by simp [hb]
  have hrl : repl.length = b.end.val - b.start.val := by
    have := congrArg List.length hr
    simp only [List.slice, List.length_take, List.length_drop] at this
    have : repl.length = repl.val.length := rfl
    have : text.length = text.val.length := rfl
    omega
  unfold block.splice
  apply WP.spec_bind (Pₘ := fun r : alloc.vec.Vec U8 × Usize =>
    r.1.val = text.val.take b.end.val ∧ r.2 = b.end)
  · unfold block.splice_loop
    apply loop.spec_decr_nat (fun s : alloc.vec.Vec U8 × Usize × Usize => 1 - s.2.2.val)
      (fun s => (s.2.2.val = 0 ∧ s.1.val = [] ∧ s.2.1.val = 0) ∨
        (s.2.2.val = 1 ∧ s.1.val = text.val.take b.end.val ∧ s.2.1 = b.end))
    · rintro ⟨out, cursor, i⟩ hinv
      dsimp only
      unfold block.splice_loop.body
      dsimp only
      rcases hinv with ⟨hi, hout, hc⟩ | ⟨hi, hout, hc⟩
      · have hlt : i < blocks.len := by scalar_tac
        simp only [hlt, ↓reduceIte]
        apply WP.spec_bind (Slice.index_usize_spec blocks i (by scalar_tac))
        intro x hx
        have hxb : x = b := by rw [hx]; simp [hb, hi]
        clear hx
        subst hxb
        have hc0 : cursor.val = 0 := hc
        have htl : text.length = text.val.length := rfl
        apply WP.spec_bind (core.slice.index.SliceIndexRangeUsizeSlice.index.step_spec
          { start := cursor, «end» := x.start } text (by dsimp only; scalar_tac)
          (by dsimp only; scalar_tac))
        intro sl ⟨hsl, _⟩
        have hsll : sl.val.length = x.start.val := by
          rw [hsl]
          simp only [List.slice, List.length_take, List.length_drop]
          omega
        have hol : out.length = 0 := by
          have : out.length = out.val.length := rfl
          rw [this, hout]; rfl
        apply WP.spec_bind (extend_spec out sl (by
          have : sl.length = sl.val.length := rfl
          omega))
        intro out1 hout1
        have hi0 : i = 0#usize := by scalar_tac
        simp only [hi0, ↓reduceIte]
        have ho1 : out1.length = x.start.val := by
          have : out1.length = out1.val.length := rfl
          rw [this, hout1, hout, List.nil_append, hsll]
        apply WP.spec_bind (extend_spec out1 repl (by omega))
        intro out2 hout2
        apply WP.spec_bind (Usize.add_spec (by scalar_tac))
        intro i2 hi2
        simp only [WP.spec_ok]
        refine ⟨Or.inr ⟨by scalar_tac, ?_, trivial⟩, by scalar_tac⟩
        rw [hout2, hout1, hout, hsl, ← hr]
        simp only [List.slice, hc0, List.drop_zero, Nat.sub_zero, List.nil_append]
        rw [← List.take_append_drop x.start.val (text.val.take x.end.val)]
        rw [List.take_take, List.drop_take, Nat.min_eq_left hs]
      · have hlt : ¬ i < blocks.len := by scalar_tac
        simp only [hlt, ↓reduceIte, WP.spec_ok]
        exact ⟨hout, hc⟩
    · left; simp
  · rintro ⟨acc, cursor⟩ ⟨hacc, hc⟩
    simp only [Std.uncurry]
    subst hc
    apply WP.spec_bind (core.slice.index.SliceIndexRangeFromUsizeSlice.index.step_spec
      { start := b.end } text (by simpa using he))
    intro tail ⟨htail, _⟩
    obtain ⟨v, hv, hvv⟩ := spec_exists_ok (extend_spec acc tail (by
      have : acc.length = acc.val.length := rfl
      have h2 : tail.length = tail.val.length := rfl
      have h3 : text.length = text.val.length := rfl
      rw [this, h2, hacc, htail]
      simp only [List.length_take, List.length_drop]
      omega))
    rw [hv, WP.spec_ok, hvv, hacc, htail, List.take_append_drop]

/-- K5: upserting an upsert's output again, with the same inputs, returns it
unchanged. -/
theorem upsert_idempotent (text owner : Slice U8) (style : block.Style) (digest : ids.Digest)
    (mode : U32) (payload : Slice U8) (out : alloc.vec.Vec U8)
    (h : block.upsert text owner style digest mode payload = ok (.Ok out)) :
    block.upsert (alloc.vec.Vec.deref out) owner style digest mode payload = ok (.Ok out) := by
  obtain ⟨blocks, newline, normalized, sha, rendered, _, _, _, hnorm, hsha, hrender, _,
    hsettled⟩ := upsert_ok text owner style digest mode payload out h
  obtain ⟨blocks2, b, hscan2, hb2, hord2, hs, he, hslice, hnl2, _, _, _⟩ :=
    settled_ok _ _ _ _ _ _ _ hsettled
  obtain ⟨v, hv, hvval⟩ := spec_exists_ok (splice_single (alloc.vec.Vec.deref out)
    (alloc.vec.Vec.deref rendered) (alloc.vec.Vec.deref blocks2) b (by simpa using hb2) hs he
    (by simpa using hslice))
  have hvout : v = out := by
    apply alloc.vec.Vec.ext
    simpa using hvval
  subst hvout
  have hz : ¬ (alloc.vec.Vec.len blocks2 = 0#usize) := by
    intro hz
    have := congrArg UScalar.val hz
    simp [hb2] at this
  unfold block.upsert
  simp only [hscan2, bind_ok, hord2, ↓reduceIte, hnl2, hnorm, hsha, hrender, hz, hv, hsettled]

/-- The round trip: the output holds exactly one block of the owner, which is
the rendered block; its marker records the digest's sha digits and the mode,
and its content normalizes to the payload's normalization — so the next scan
sees it intact, never edited. -/
theorem upsert_round_trip (text owner : Slice U8) (style : block.Style) (digest : ids.Digest)
    (mode : U32) (payload : Slice U8) (out : alloc.vec.Vec U8)
    (h : block.upsert text owner style digest mode payload = ok (.Ok out)) :
    ∃ blocks b normalized sha newline rendered,
      block.scan (alloc.vec.Vec.deref out) owner = ok (.Ok blocks) ∧ blocks.val = [b] ∧
      block.normalize payload = ok normalized ∧ block.mark digest = ok sha ∧
      block.newline_of text = ok newline ∧
      block.render style owner sha mode newline (alloc.vec.Vec.deref normalized) = ok rendered ∧
      out.val.slice b.start.val b.end.val = rendered.val ∧
      out.val.slice b.sha.val (b.sha.val + 16) = sha.val ∧ b.mode = some mode ∧
      ∃ (content : Slice U8) (normal : alloc.vec.Vec U8),
        content.val = out.val.slice b.content_start.val b.content_end.val ∧
        block.normalize content = ok normal ∧ normal.val = normalized.val := by
  obtain ⟨blocks, newline, normalized, sha, rendered, _, _, hnl, hnorm, hsha, hrender, _,
    hsettled⟩ := upsert_ok text owner style digest mode payload out h
  obtain ⟨blocks2, b, hscan2, hb2, _, _, _, hslice, _, hshab, hmode, content, normal, hc, hn,
    hnv⟩ := settled_ok _ _ _ _ _ _ _ hsettled
  exact ⟨blocks2, b, normalized, sha, newline, rendered, hscan2, hb2, hnorm, hsha, hnl, hrender,
    by simpa using hslice, by simpa using hshab, hmode, content, normal,
    by simpa using hc, hn, by simpa using hnv⟩


/-! ## K1 for blocks: the decision -/

/-- Decisions that change the hosting file. -/
def Writes : block.BlockDecision → Prop
  | .Write => True
  | .Remove => True
  | .Satisfied => False
  | .Preserve => False
  | .Keep => False
  | .Absent => False
  | .Refuse _ => False

theorem same_digest_ok {a : Option ids.Digest} {b : ids.Digest}
    (h : block.same_digest a b = ok true) : a = some b := by
  cases a with
  | none => simp [block.same_digest] at h
  | some a =>
    simp only [block.same_digest] at h
    obtain ⟨r, hr, hp⟩ := spec_exists_ok (Digest.same_spec a b)
    rw [hr] at h
    simp only [Result.ok.injEq] at h
    subst h
    simp [hp.mp rfl]

/-- `mode_conflict` returned false: every recorded mode agrees with `live`. -/
theorem mode_conflict_false (record : Option block.BlockRecord) (blocks : Slice block.Seen)
    (live : U32) (h : block.mode_conflict record blocks live = ok false) :
    (∀ s ∈ blocks.val, ∀ m, s.mode = some m → m = live) ∧
      (∀ r m, record = some r → r.preserved = false → r.mode = some m → m = live) := by
  unfold block.mode_conflict block.mode_conflict_loop at h
  refine loop_ok (fun i : Usize => blocks.val.length - i.val)
    (fun i : Usize => ∀ j (hj : j < blocks.val.length), j < i.val →
      ∀ m, blocks.val[j].mode = some m → m = live)
    (fun r : Bool => r = false → (∀ s ∈ blocks.val, ∀ m, s.mode = some m → m = live) ∧
      (∀ r m, record = some r → r.preserved = false → r.mode = some m → m = live))
    _ ?_ 0#usize false (by intro j _ hj; simp at hj) h rfl
  intro i r hinv hr
  unfold block.mode_conflict_loop.body at hr
  dsimp only at hr
  by_cases hlt : i < blocks.len
  · simp only [hlt, ↓reduceIte, bind_eq_ok] at hr
    obtain ⟨x, hx, b, hb, hr⟩ := hr
    have hx' := index_usize_ok hx
    have hlt' : i.val < blocks.val.length := by scalar_tac
    split_ifs at hr with hbt
    · simp only [Result.ok.injEq] at hr; subst hr; simp
    · rw [bind_eq_ok] at hr
      obtain ⟨i2, hi2, hr⟩ := hr
      simp only [Result.ok.injEq] at hr
      subst hr
      have hi2' := add_ok hi2
      refine ⟨fun j hj hji m hm => ?_, by scalar_tac⟩
      by_cases hjj : j < i.val
      · exact hinv j hj hjj m hm
      · have hji' : j = i.val := by scalar_tac
        subst hji'
        have hxj : blocks.val[i.val] = x := by
          simpa [List.getElem?_eq_getElem hlt'] using hx'
        rw [hxj] at hm
        unfold block.other_mode at hb
        rw [hm] at hb
        simp only [Result.ok.injEq] at hb
        subst hb
        simp only [bne_iff_ne, ne_eq, Decidable.not_not] at hbt
        exact hbt
  · simp only [hlt, ↓reduceIte] at hr
    have hall : ∀ s ∈ blocks.val, ∀ m, s.mode = some m → m = live := by
      intro s hs m hm
      obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hs
      exact hinv j hj (by scalar_tac) m hm
    cases record with
    | none =>
      simp only [Result.ok.injEq] at hr
      subst hr
      intro _
      exact ⟨hall, by simp⟩
    | some rc =>
      dsimp only at hr
      by_cases hp : rc.preserved = true
      · simp only [hp, ↓reduceIte, Result.ok.injEq] at hr
        subst hr
        intro _
        refine ⟨hall, ?_⟩
        intro r m h1 h2
        simp only [Option.some.injEq] at h1
        subst h1
        simp [hp] at h2
      · simp only [hp, Bool.false_eq_true, ↓reduceIte] at hr
        cases hm : rc.mode with
        | none =>
          simp only [hm, Result.ok.injEq] at hr
          subst hr
          intro _
          refine ⟨hall, ?_⟩
          intro r m h1 _ h3
          simp only [Option.some.injEq] at h1
          subst h1
          simp [hm] at h3
        | some md =>
          simp only [hm, Result.ok.injEq] at hr
          subst hr
          intro hf
          refine ⟨hall, ?_⟩
          intro r m h1 _ h3
          simp only [Option.some.injEq] at h1
          subst h1
          rw [hm] at h3
          simp only [Option.some.injEq] at h3
          subst h3
          simp only [bne_eq_false_iff_eq] at hf
          exact hf

/-- What `intact` checked when it accepted a block. -/
theorem intact_true (r : block.BlockRecord) (blocks : Slice block.Seen) (live : U32)
    (h : block.intact r blocks live = ok true) :
    r.preserved = false ∧ ∃ s, blocks.val = [s] ∧ s.marked = true ∧ r.hash = some s.content ∧
      (∀ m, s.mode = some m → m = live) ∧
      (r.mode = some live ∨ (r.mode = none ∧ s.mode = some live)) := by
  unfold block.intact at h
  dsimp only at h
  simp only [bind_eq_ok, ite_ok_true, ite_false_ok_true, Std.uncurry] at h
  obtain ⟨hp, h1, x, hx, pr, hpr, ma, hma, b, hb, hb', hma', b1, hb1, hb1', hmk⟩ := h
  subst hb' hma' hb1'
  have hlen1 : blocks.val.length = 1 := by
    simp only [bne_iff_ne, ne_eq, Decidable.not_not] at h1
    have := congrArg UScalar.val h1; simpa using this
  obtain ⟨s, hs⟩ : ∃ s, blocks.val = [s] := List.length_eq_one_iff.mp hlen1
  have hxs : x = s := by
    have := index_usize_ok hx
    simp [hs] at this
    exact this.symm
  subst hxs
  simp only [Result.ok.injEq] at hmk
  have hd := same_digest_ok hb1
  have hpr1 : pr.1 = x := by
    cases hrm : r.mode <;> rw [hrm] at hpr <;> simp only [Result.ok.injEq] at hpr <;> rw [← hpr]
  rw [hpr1] at hd hmk hma
  refine ⟨by simpa using hp, x, hs, hmk, hd, ?_, ?_⟩
  · intro m hm
    rw [hm] at hma
    simp only [Result.ok.injEq, decide_eq_true_eq] at hma
    exact hma
  · have hsm := same_mode_ok hb
    cases hrm : r.mode with
    | none =>
      rw [hrm] at hpr
      simp only [Result.ok.injEq] at hpr
      rw [← hpr] at hsm
      exact Or.inr ⟨rfl, hsm⟩
    | some v =>
      rw [hrm] at hpr
      simp only [Result.ok.injEq] at hpr
      rw [← hpr] at hsm
      exact Or.inl hsm

/-- What a deploy decision can be, and when it preserves. -/
theorem deploy_facts (record : Option block.BlockRecord) (host : block.Host)
    (blocks : Slice block.Seen) (desired : ids.Digest) (mode : U32) (d : block.BlockDecision)
    (h : block.deploy record host blocks desired mode = ok d) :
    (d = .Write ∨ d = .Satisfied ∨ d = .Preserve ∨ d = .Refuse .Foreign) ∧
    (host = .Foreign → d = .Refuse .Foreign) ∧
    (∀ live, host = .File live → blocks.val ≠ [] → d ≠ .Preserve →
      block.mode_conflict record blocks live = ok false) := by
  unfold block.deploy at h
  cases host with
  | Absent =>
    simp only [Result.ok.injEq] at h
    subst h
    simp
  | Foreign =>
    simp only [Result.ok.injEq] at h
    subst h
    simp
  | File live =>
    dsimp only at h
    by_cases hpos : blocks.len > 0#usize
    · simp only [hpos, ↓reduceIte, bind_eq_ok] at h
      obtain ⟨c, hc, h⟩ := h
      cases c with
      | true =>
        simp only [↓reduceIte, Result.ok.injEq] at h
        subst h
        simp
      | false =>
        simp only [Bool.false_eq_true, ↓reduceIte] at h
        refine ⟨?_, by simp, fun l hl _ _ => ?_⟩
        · split_ifs at h
          · rw [bind_eq_ok] at h
            obtain ⟨b, _, h⟩ := h
            split_ifs at h <;> simp only [Result.ok.injEq] at h <;> subst h <;> simp
          · simp only [Result.ok.injEq] at h; subst h; simp
        · simp only [block.Host.File.injEq] at hl
          subst hl
          exact hc
    · simp only [hpos, ↓reduceIte] at h
      refine ⟨?_, by simp, fun l hl hb _ => ?_⟩
      · split_ifs at h
        · rw [bind_eq_ok] at h
          obtain ⟨b, _, h⟩ := h
          split_ifs at h <;> simp only [Result.ok.injEq] at h <;> subst h <;> simp
        · simp only [Result.ok.injEq] at h; subst h; simp
      · exfalso
        apply hb
        have : blocks.len.val = 0 := by scalar_tac
        simpa using this

/-- K1 for blocks: a block is removed only by a removal, and only if it is
intact — the hosting path is a file, the record is gripsack's write (not
preserved drift), there is exactly one block of the owner, its marker's sha
digits name its content, its content is the recorded content, and neither the
marker nor the record names a mode other than the live file's. -/
theorem decide_block_remove_intact (record : Option block.BlockRecord) (host : block.Host)
    (blocks : Slice block.Seen) (intent : block.BlockIntent)
    (h : block.decide_block record host blocks intent = ok .Remove) :
    intent = .Remove ∧ ∃ r live s, record = some r ∧ host = .File live ∧ blocks.val = [s] ∧
      r.preserved = false ∧ s.marked = true ∧ r.hash = some s.content ∧
      (∀ m, s.mode = some m → m = live) ∧
      (r.mode = some live ∨ (r.mode = none ∧ s.mode = some live)) := by
  unfold block.decide_block at h
  cases intent with
  | Deploy desired mode =>
    rcases (deploy_facts _ _ _ _ _ _ h).1 with h1 | h1 | h1 | h1 <;> cases h1
  | Restore retained recorded mode =>
    simp only [bind_eq_ok] at h
    obtain ⟨d, hd, b, hb, h⟩ := h
    rcases (deploy_facts _ _ _ _ _ _ hd).1 with h1 | h1 | h1 | h1 <;> subst h1 <;>
      split_ifs at h <;> simp at h
  | Remove =>
    refine ⟨rfl, ?_⟩
    cases host with
    | Absent => simp at h
    | Foreign => simp at h
    | File live =>
      dsimp only at h
      split_ifs at h
      · simp at h
      cases record with
      | none => simp at h
      | some r =>
        dsimp only at h
        rw [bind_eq_ok] at h
        obtain ⟨b, hb, h⟩ := h
        cases b with
        | false => simp at h
        | true =>
          obtain ⟨hp, s, hs, hm, hh, hmode, hrm⟩ := intact_true _ _ _ hb
          exact ⟨r, live, s, rfl, rfl, hs, hp, hm, hh, hmode, hrm⟩

/-- A hosting path that is not a file (a link, a directory) is never spliced
or emptied, whatever the intent. -/
theorem decide_block_foreign_untouched (record : Option block.BlockRecord)
    (blocks : Slice block.Seen) (intent : block.BlockIntent) (d : block.BlockDecision)
    (h : block.decide_block record .Foreign blocks intent = ok d) : ¬ Writes d := by
  unfold block.decide_block at h
  cases intent with
  | Deploy desired mode =>
    rw [(deploy_facts _ _ _ _ _ _ h).2.1 rfl]
    simp [Writes]
  | Restore retained recorded mode =>
    simp only [bind_eq_ok] at h
    obtain ⟨d', hd, b, hb, h⟩ := h
    have := (deploy_facts _ _ _ _ _ _ hd).2.1 rfl
    subst this
    split_ifs at h <;> simp only [Result.ok.injEq] at h <;> subst h <;> simp [Writes]
  | Remove =>
    simp only [Result.ok.injEq] at h
    subst h
    simp [Writes]

/-- Conflicting mode evidence — a block's marker, or gripsack's (non-drift)
record, naming a mode other than the live file's — never writes: the whole
file is preserved, or a removal keeps the block. -/
theorem decide_block_mode_evidence_preserves (record : Option block.BlockRecord) (live : U32)
    (blocks : Slice block.Seen) (intent : block.BlockIntent) (d : block.BlockDecision)
    (hb : blocks.val ≠ [])
    (hc : (∃ s ∈ blocks.val, ∃ m, s.mode = some m ∧ m ≠ live) ∨
      (∃ r m, record = some r ∧ r.preserved = false ∧ r.mode = some m ∧ m ≠ live))
    (h : block.decide_block record (.File live) blocks intent = ok d) : ¬ Writes d := by
  have agree : block.mode_conflict record blocks live = ok false → False := by
    intro hf
    obtain ⟨h1, h2⟩ := mode_conflict_false _ _ _ hf
    rcases hc with ⟨s, hs, m, hm, hne⟩ | ⟨r, m, hr, hp, hm, hne⟩
    · exact hne (h1 s hs m hm)
    · exact hne (h2 r m hr hp hm)
  cases intent with
  | Deploy desired mode =>
    unfold block.decide_block at h
    intro hw
    obtain ⟨_, _, hpres⟩ := deploy_facts _ _ _ _ _ _ h
    apply agree (hpres live rfl hb ?_)
    intro hp; subst hp; exact hw
  | Restore retained recorded mode =>
    unfold block.decide_block at h
    simp only [bind_eq_ok] at h
    obtain ⟨d', hd, b, _, h⟩ := h
    obtain ⟨_, _, hpres⟩ := deploy_facts _ _ _ _ _ _ hd
    intro hw
    have hd' : d' ≠ .Preserve := by
      intro hp; subst hp
      split_ifs at h <;> simp only [Result.ok.injEq] at h <;> subst h <;> exact hw
    exact agree (hpres live rfl hb hd')
  | Remove =>
    intro hw
    have hrem : d = .Remove := by
      unfold block.decide_block at h
      dsimp only at h
      split_ifs at h
      · simp only [Result.ok.injEq] at h; subst h; simp [Writes] at hw
      cases record with
      | none => simp only [Result.ok.injEq] at h; subst h; simp [Writes] at hw
      | some r =>
        dsimp only at h
        rw [bind_eq_ok] at h
        obtain ⟨b, _, h⟩ := h
        split_ifs at h <;> simp only [Result.ok.injEq] at h <;> subst h
        · rfl
        · simp [Writes] at hw
    subst hrem
    obtain ⟨_, r, l, s, hr, hl, hs, _, _, _, hmode, hrm⟩ :=
      decide_block_remove_intact _ _ _ _ h
    simp only [block.Host.File.injEq] at hl
    subst hl
    rcases hc with ⟨s', hs', m, hm, hne⟩ | ⟨r', m, hr', hp, hm, hne⟩
    · rw [hs] at hs'
      simp only [List.mem_singleton] at hs'
      subst hs'
      exact hne (hmode m hm)
    · rw [hr] at hr'
      simp only [Option.some.injEq] at hr'
      subst hr'
      rcases hrm with hrm | ⟨hrm, _⟩ <;> rw [hrm] at hm <;> simp at hm
      exact hne hm.symm

/-- Rollback restores a block only from a retained payload that is the record:
writing it, or finding it already in place, needs the retained payload's
digest to be the target generation's recorded digest. -/
theorem decide_block_restore_exact (record : Option block.BlockRecord) (host : block.Host)
    (blocks : Slice block.Seen) (retained : ids.Digest) (recorded : Option ids.Digest)
    (mode : U32) (d : block.BlockDecision)
    (h : block.decide_block record host blocks (.Restore retained recorded mode) = ok d)
    (hd : d = .Write ∨ d = .Satisfied) : recorded = some retained := by
  unfold block.decide_block at h
  simp only [bind_eq_ok] at h
  obtain ⟨d', hd', b, hb, h⟩ := h
  cases b with
  | true => exact same_digest_ok hb
  | false =>
    simp only [Bool.false_eq_true, ↓reduceIte] at h
    exfalso
    rcases hd with rfl | rfl <;> cases d' <;> simp at h

end Gripsack.Kernel.Block
