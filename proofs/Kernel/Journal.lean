import Extracted.GripsackKernel

/-!
# Journal publication and mutation authority (plan/0056 F1)

Theorems over the Aeneas translation of `gripsack_kernel::journal`
(`Extracted/GripsackKernel.lean`). These are the former Verus contracts of
`RecordPublication::{new, action, acknowledge, finish}`, restated over the
extracted functions. (`admit_mutation` is gone: the journal-run machine's P1,
`Kernel/JournalRun.lean`, states when a mutation may follow.)
-/

open Aeneas Aeneas.Std Result
open gripsack_kernel

namespace Gripsack.Kernel

/-- `RecordRole::same` is equality. -/
theorem RecordRole.same_spec (a b : journal.RecordRole) :
    journal.RecordRole.same a b ⦃ r => r ↔ a = b ⦄ := by
  cases a <;> cases b <;> simp [journal.RecordRole.same]

/-- `PublicationAction::same` is equality. -/
theorem PublicationAction.same_spec (a b : journal.PublicationAction) :
    journal.PublicationAction.same a b ⦃ r => r ↔ a = b ⦄ := by
  cases a <;> cases b <;> simp [journal.PublicationAction.same]

/-- The publication order: stage, sync the file, publish the name, sync the
namespace, complete. Nothing follows completion or refusal. -/
theorem PublicationAction.successor_spec :
    journal.PublicationAction.successor .Stage = ok .SyncFile ∧
    journal.PublicationAction.successor .SyncFile = ok .PublishName ∧
    journal.PublicationAction.successor .PublishName = ok .SyncNamespace ∧
    journal.PublicationAction.successor .SyncNamespace = ok .Complete ∧
    journal.PublicationAction.successor .Complete = ok .Refused ∧
    journal.PublicationAction.successor .Refused = ok .Refused := by
  simp [journal.PublicationAction.successor]

/-- A fresh publication expects `Stage` and keeps its role. -/
theorem RecordPublication.new_spec (role : journal.RecordRole) :
    journal.RecordPublication.new role ⦃ p => p.role = role ∧ p.next = .Stage ⦄ := by
  simp [journal.RecordPublication.new]

/-- `action` reports the expected next effect. -/
theorem RecordPublication.action_spec (p : journal.RecordPublication) :
    journal.RecordPublication.action p ⦃ a => a = p.next ⦄ := by
  simp [journal.RecordPublication.action]

/-- The acknowledgement contract (former Verus `ensures`): the role is kept;
the acknowledgement is accepted iff the effect succeeded and was the expected
next action other than completion or refusal; on acceptance the publication
advances to that action's successor, otherwise it is refused for good. -/
theorem RecordPublication.acknowledge_spec (p : journal.RecordPublication)
    (action : journal.PublicationAction) (succeeded : Bool) :
    journal.RecordPublication.acknowledge p action succeeded ⦃ accepted p' =>
      p'.role = p.role ∧
      (accepted ↔ (succeeded ∧ action = p.next ∧ action ≠ .Complete ∧ action ≠ .Refused)) ∧
      (accepted → journal.PublicationAction.successor action = ok p'.next) ∧
      (¬ accepted → p'.next = .Refused) ⦄ := by
  obtain ⟨role, next⟩ := p
  cases succeeded <;> cases action <;> cases next <;>
    simp [journal.RecordPublication.acknowledge, journal.PublicationAction.same,
      journal.PublicationAction.successor]

/-- `finish` yields a durable record iff the publication completed, and the
record carries the publication's role. -/
theorem RecordPublication.finish_spec (p : journal.RecordPublication) :
    journal.RecordPublication.finish p ⦃ r =>
      (r.isSome ↔ p.next = .Complete) ∧
      (∀ d, r = some d → d.role = p.role) ⦄ := by
  obtain ⟨role, next⟩ := p
  cases next <;> simp [journal.RecordPublication.finish, journal.PublicationAction.same]

end Gripsack.Kernel
