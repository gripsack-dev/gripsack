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
<motivation; link the plan doc section that justifies the design>

## how
<key implementation decisions; anything a reviewer would trip on>

## evidence
<proof the change works — see below>

## checklist
- [ ] compose gates green (test, pytest, e2e)
- [ ] IR touched? schema + rust + python in this PR (gripsack-ir skill)
- [ ] new flow? e2e unskipped/added in this PR
- [ ] plan docs updated in this PR
```

## Evidence contract

- **Behavior changes**: e2e output showing the flow (apply/rollback
  transcript against a fixture env repo), pasted in a fenced block.
- **CLI output changes**: paste the before/after terminal text.
- **Visual/demo-worthy**: extend `demos/demo.tape`; the demo workflow
  re-renders (gripsack-demo-capture skill). Don't commit one-off GIFs.
- **IR changes**: the JSON diff of emitted IR for the same module,
  before/after.

## Process

- Branch `<feat|fix|chore>/<slug>`, commits grouped by theme.
- Title mirrors the dominant commit type (`feat:`, `fix:`, `chore:`,
  `plan:`).
- `main` is protected: `test` check required. The owner merges with
  admin override.
- Self-review before requesting review: no stray debug output, no
  scratch files, gates actually run (not aspirational checkboxes).
