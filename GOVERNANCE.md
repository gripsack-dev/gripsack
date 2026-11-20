# Governance

The owner decides Gripsack's scope, accepts or declines work, sets release milestones, and decides what ships. Contributors and agents may report problems, draft RFCs, review proposals, and implement accepted work. **Only the owner can accept or reject an RFC.** Merging a draft, putting an issue in a milestone, or passing qualification does not accept a design.

## Triage

A [GitHub issue](https://github.com/gripsack-dev/gripsack/issues) tracks a problem, the agreed outcome, and the work left to do. Open one before substantial work; for exceptions and private security reporting, see [Contributing](CONTRIBUTING.md).

| Label | How to use it |
| --- | --- |
| `needs-triage` | New issue awaiting an owner decision. Remove on acceptance or decline. |
| `accepted` | The owner wants this outcome. If it needs an RFC, implementation still waits for the owner to accept the design. |
| `declined` | The owner decided not to pursue the issue. Record why and close as not planned, not completed. |
| `blocked` | Work cannot proceed. Link the dependency or state what would unblock it. |
| `needs-owner` | State the specific question only the owner can settle. Remove when the answer is recorded. |
| `needs-rfc` | A design decision is required before implementation. |
| `rfc` | The issue tracks a specific RFC; link the file from the issue and the issue from the RFC. Replace `needs-rfc` when the RFC is linked; its frontmatter records the decision. |
| `needs-proof` | A ring 0 decision or ring 1 protocol needs Lean proof over the Aeneas translation of the shipped Rust. Remove when the obligation is discharged, not when a test passes. |

Use a type label (`bug`, `enhancement`, or `documentation`). Add an area when it helps route work: `area/kernel` (includes proofs), `area/host`, `area/ir` (includes the TypeScript SDK), `area/exec`, `area/fetch`, `area/cli`, or `area/release`. A blocker or area is not a triage decision.

## Scheduling and closure

The owner assigns accepted work to a [release milestone](https://github.com/gripsack-dev/gripsack/milestones) for a planned release, or to `Later` if no release is selected. An unassigned issue has no release commitment. Do not invent a release or due date. When a release is tagged, move unfinished issues to the agreed milestone or `Later`, note why on the issues, then close the release milestone.

Use `Refs #N` for partial work and `Closes #N` only when the agreed outcome lands on `main`. RFC acceptance is a decision, not evidence of delivery. Keep design decisions, implementation state, and supersession links in the [RFC registry and lifecycle](docs/rfcs/README.md); keep release-facing changes in [CHANGELOG.md](CHANGELOG.md).
