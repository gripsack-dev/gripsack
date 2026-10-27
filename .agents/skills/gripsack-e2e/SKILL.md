---
name: gripsack-e2e
description: Write gripsack flow tests — fixture env repos, sandboxed HOME, offline fixtures, golden IR corpus
---

# Writing e2e flow tests

E2E defends the observable contract of `grip` (plan/0003 §5): apply,
incremental reuse, scoped profile apply, rollback, drift detection. It runs
the **real binary** against the **real TypeScript frontend** under the
sandboxed Deno eval (plan/0013) — no mocks of either side.

## Harness conventions

- Everything under a `tmp_path`: `HOME` is redirected there (plus
  `GRIPSACK_HOME`), so a test can never touch the developer's real
  profile. Assert against the filesystem, not stdout snapshots.
- **Trust gate**: `grip`, `run_grip` and `start_grip` explicitly inspect and
  approve the source/policy digests of disposable fixtures under the sandbox.
  They never infer authority from CI or use an application bypass. Gate tests
  pass `approve=False` and perform their own inspect/add steps. Fault controls
  are stripped only from fixture approval, not the command under test.
- **Offline only.** Sources are `file://` fixture tarballs built in the
  test (`tarfile` module) or tiny local git repos (`git init`). Network
  in e2e is a bug — CI has no credentials and flakes are unacceptable.
  Deno is inherited from PATH (or `GRIPSACK_DENO`); nothing downloads.
- Fixture repositories use root `gripsack.ts` and `defineWorkspace`.
  `conftest.make_workspace_repo` writes current output values under
  `outputs/<name>.ts`; `refresh_workspace(repo)` composes those values and
  adopt-generated `profiles/*.ts` deterministically. Use `remove_output`
  to undeclare an output. These helpers never translate a legacy DSL.
- The binary path comes from `GRIPSACK_BIN` (default
  `target/debug/grip`); inside docker the gate stage has already
  compiled it — never rebuild from e2e.

## Golden IR corpus (test_golden.py)

Each workspace under `fixtures/envs/` is evaluated by the SDK driver with fixed
inputs and the core's deny-by-default grant profile. This direct SDK harness
does not claim captured-source approval or process receipts; those are real-CLI
acceptance cases. The emitted envelope is retained under `fixtures/golden/`.

- Preserve declaration spans and Bash `line_map`; normalize only the disposable
  absolute checkout prefix. Dedicated CLI cases check rendered source mapping.
- Regenerate through direct SDK emission when the contract changes. The owner's
  no-corpus-replay instruction remains active: do not invoke golden test
  functions or describe generation as a passing corpus campaign.

## Assertions that matter

- Store paths exist and stay immutable between applies; unchanged packages and
  profiles reuse their retained payloads.
- A changed apply publishes once; a satisfied apply retains its generation.
- Rollback restores byte-identical config content, not just paths.
- `tracked-copy` drift: modify the deployed file, re-apply, assert the
  drift report (keep/adopt/restore paths).

## Scope

Do not add skipped placeholders for unimplemented features. Test the current
behavior, including refusal before effects where a capability is unavailable.
Fuzz engines and saved-corpus replay require a separate owner instruction.
