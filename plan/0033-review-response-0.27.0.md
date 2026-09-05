# 0033 - Review response: the 0.27.0 external review

Status: **implemented in 0.29.0** (fixes), roadmap for the rest.
Source: a stricter external review of 0.27.0 (commit `ee63248`).
Every reproduced claim was re-verified against the tree before acting.

## Already fixed before this review landed (0.28.0, plan/0031 + the typing pass)

The reviewer tested the 0.27.0 binary; `main` had moved:

| Finding | Fixed by |
|---|---|
| Executable rollback leaves the newer content | The typing pass: rollback's intact compares were mixing the journal-intent and manifest identity domains — exactly the reviewer's diagnosis. Now compare manifest records; e2e `test_exec_copy_rollback_restores_v1` |
| Private merge file (0600) fails rollback with a spurious "changed" | Same class: the merge splice's precondition was bytes-only against a mode-aware journal identity. Mode-aware now; e2e `test_merge_into_private_file_rolls_back` |
| "Replace interchangeable identity strings with distinct types" | Shipped: `PayloadHash` / `BytesHash` / `FileIdentity`, `ObjectIdentity`/`Intended` at the journal boundary |

## Adopted, fixed in 0.29.0

| # | Finding | Fix |
|---|---|---|
| R1 | Take-over of a 0600 file lands it 0644 | Take-over PRESERVES the live mode and records it as the entry's mode (adoption is not a fresh deploy; 0031's "absorbed = managed mode" was wrong for confidentiality). Repo-driven exec changes still apply afterward. |
| R2 | Prior blobs land 0644 — a 0600 file's backup is world-readable | Prior blob store writes 0600, `$GRIPSACK_HOME/prior/` is 0700. Blobs are user file bytes; treat as secrets. |
| R3 | `node_modules/@gripsack/core` symlink enlarges eval's read grant without validation | The pin grant requires the resolved target to BE a package (`package.json` with `name = "@gripsack/core"`); otherwise no grant and the pin is inert. |
| R4 | Step `needs` validated but execution ignores it | Steps execute in `needs` order (intra-module topo, cycle = E120). |
| R5 | `grip plan` omits run-step mutations; skips linters | Plan runs the same validation pipeline as check (linters included) and never claims "nothing would change" for a module with opaque run steps — they mark "may change". |
| R6 | Adopt codegen: digit-leading idents, unescaped string literals | `ident` prefixes digit-leading names; every interpolated string is JSON-escaped; E116-invalid names are refused with a rename hint. |
| R7 | Lineage model doesn't cover the owned-link branch; mode change transferred authority | The explorer gains the owned branch AND a mode-change action: changing ownership mode over preserved drift must preserve, never overwrite. Production: a preserved-drift previous entry counts as foreign for the owned guard. |

## Adopted, deferred (roadmap)

- **One shared operation list for plan/apply/rollback** — the
  reviewer's central architectural point, and the shape 0007/0026
  have been converging on. Plan renders the same operation list apply
  executes. Big refactor, AFTER the soak cycle the reviewer also
  asks for. Note the honest contract: plan previews computed intent;
  apply still revalidates observations before writing (external state
  is never fully predictable — the journal precondition stays).
- **Fetch memory budget** — streaming archive extraction + an
  acquisition concurrency cap separate from the module parallelism.
  Roadmap, after the transaction work.
- **Drift reconciliation UX** — the reviewer's "next product
  capability" is the roadmap's `grip resolve` item; bumped in
  priority text (double-endorsed now).

## Pushed back

- **Machine-local setting for the pin grant** — the grant's defect
  was the missing validation (R3, adopted). A setting would break
  every `npm link`-style pin workflow for zero confinement gain: a
  fake `@gripsack/core` package carries arbitrary code either way,
  and eval remains no-net/no-env/no-run/no-write regardless of the
  read set. The trust gate (`grip trust`) is the machine-local
  boundary; the pin rule rides it.
- **"Plan can omit real mutations" as fully solvable** — adopted the
  fidelity fixes (R5), but plan is a preview computed from observed
  state; apply's preconditions are the authority. The shared
  operation list (roadmap) narrows the gap honestly rather than
  claiming it away.
- **Mission narrowing / hero CTA / install order** — owner's call,
  now with an external second opinion (fourth data point). The
  factual website fixes landed without ceremony (chezmoi row, h1,
  social meta, copy-button error handling); the framing decisions
  stay with the owner.
