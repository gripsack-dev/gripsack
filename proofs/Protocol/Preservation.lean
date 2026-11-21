import Storage

/-!
# The leaf settlement cut (#364)

`Preserve` gets no durable fact from `decide`: even a genuinely foreign
observation can be entirely volatile. These are local consequences of the
successful host barriers, on an arbitrary pre-barrier `FS`. `All` includes
both the kill-visible image and *every* admitted power image. Kill alone
does not discharge any outstanding durability choice.

`n` is the inode of the open file whose bytes and mode were rechecked (not
an inode obtained by reopening the leaf after checking it). The name check
must still describe the binding at the parent fsync, and the rechecked
bytes/mode the file at fsync(file): the existing S5 window, not CAS. The
consecutive `Completed` steps below do not insert a concurrent writer. A
symlink's target is immutable for its inode; a namespace barrier does not
flush the contents of a regular file. Locations are relative to the pinned
parent; reachability of that parent from the admitted root is separate.

This is neither recovery command composition nor a directory theorem.
Failures, death before acknowledgement, and framing the settlement through
later journal operations belong to the cleanup cut proof (#367).
-/
namespace Gripsack.Protocol.Preservation

open Gripsack.Storage

private theorem same_pinned_inode {m p q slot h k}
    (a : PinnedFile m p slot h) (b : PinnedFile m q slot k) : h.ino = k.ino := by
  obtain ⟨hp, hh, _⟩ := a
  obtain ⟨hq, hk, _⟩ := b
  have e : p = q := Option.some.inj (hp.symm.trans hq)
  subst q
  exact congrArg Handle.ino (Option.some.inj (hh.symm.trans hk))

/-- The exact file descriptor, namespace binding, bytes *and* mode. There
is no restriction on the starting crash images or their number. -/
theorem checked_file_settlement {m f t : Machine} {l : Loc} {p slot h}
    (fd : PinnedFile m p slot h) (named : m.fs.visible.link l = some h.ino)
    (file : Completed m (.syncFile slot) f) (parent : Completed f (.syncDir l.parent) t) :
    All t.fs (fun i => i.link l = some h.ino ∧ i.read l = some (m.fs.visible.file h.ino)) := by
  cases file with
  | syncFile actual =>
    have eq := same_pinned_inode fd actual
    cases parent
    intro i hi
    have link : i.link l = some h.ino := (crash_syncDir_link (l := l) hi rfl).trans named
    have data := ((StableFile.after_syncFile _ _).syncDir (d := l.parent)) i hi
    rw [← eq] at data
    exact ⟨link, by simp [Image.read, link, data]⟩

/-- A rechecked symlink is sealed by syncing its parent, not by opening
or rewriting its target. -/
theorem checked_symlink_settlement {m t : Machine} {l : Loc} {target}
    (read : Completed m (.readLink l target) m) (parent : Completed m (.syncDir l.parent) t) :
    ∃ n, t.kind n = some (.symlink target) ∧ StableLink t.fs l (some n) := by
  cases read with
  | readLink _ named kind =>
    cases parent
    exact ⟨_, kind, by simpa [named] using StableLink.after_syncDir m.fs l⟩

/-- The other name-only `Seal` case is an already absent prior. -/
theorem checked_absence_settlement {m t : Machine} {l : Loc}
    (missing : m.fs.visible.link l = none) (parent : Completed m (.syncDir l.parent) t) :
    StableLink t.fs l none := by
  cases parent
  simpa [missing] using StableLink.after_syncDir m.fs l

end Gripsack.Protocol.Preservation
