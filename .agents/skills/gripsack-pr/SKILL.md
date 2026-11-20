---
name: gripsack-pr
description: Author gripsack PRs — template and evidence contract
---

# Authoring PRs

## Template

```
## what
<one paragraph: the change as a user of grip would observe it>

## why
<motivation; Refs #N for partial work, Closes #N only when this finishes it;
link the governing RFC if one is required>

## how
<key implementation decisions; anything a reviewer would trip on>

## evidence
<proof the change works — see below>

## checklist
- [ ] relevant compose gates identified; actual results or checks not run stated
- [ ] IR touched? schema + rust + typescript in this PR (gripsack-ir skill)
      + golden IR generated directly when corpus replay is excluded
- [ ] changed flow exercised through the actual program
- [ ] kernel/protocol change? Rust, Aeneas translation and Lean proofs updated together
- [ ] docs, changelog and applicable RFC implementation state + registry row updated
```

## Evidence contract

- **Behavior changes**: e2e output showing the flow (apply/rollback
  transcript against a fixture env repo), pasted in a fenced block.
- **CLI output changes**: paste the before/after terminal text.
- **Visual/user-facing**: explain flows with the website's animated diagrams
  (gripsack-dev.github.io) or `docs/diagrams/*.d2`, not terminal recordings.
- **IR changes**: show current emitted workspace IR and rejection of retired
  envelopes; retained storage behavior is a separate lifecycle check.

## Process

- Follow [CONTRIBUTING.md](../../../CONTRIBUTING.md): open or find the
  tracking issue before substantial work. Wait for owner acceptance before
  implementing. If an RFC is required, only the owner can accept it; a
  draft PR or merged draft does not authorize implementation. See the
  [RFC registry and lifecycle](../../../docs/rfcs/README.md).
- Branch `<feat|fix|chore>/<slug>`, commits grouped by theme.
- Title mirrors the dominant commit type (`feat:`, `fix:`, `chore:`,
  `plan:`).
- Protected `main` requires `local-practical-qualification` on the exact
  source from the owner's local Buildbot. Hosted diagnostics and a
  developer's compose run are not that status.
- Self-review before requesting review: no stray debug output, no
  scratch files, gates actually run (not aspirational checkboxes).
