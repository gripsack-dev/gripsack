---
name: gripsack-e2e
description: Write gripsack flow tests — fixture env repos, sandboxed HOME, offline fixtures, golden IR corpus
---

# Writing e2e flow tests

E2E defends the observable contract of `grip` (plan/0003 §5): apply,
incremental reuse, single-module apply, rollback, drift detection. It runs
the **real binary** against the **real TypeScript frontend** under the
sandboxed Deno eval (plan/0013) — no mocks of either side.

## Harness conventions

- Everything under a `tmp_path`: `HOME` is redirected there (plus
  `GRIPSACK_HOME`), so a test can never touch the developer's real
  profile. Assert against the filesystem, not stdout snapshots.
- **Trust gate**: `conftest.sandbox` sets `GRIPSACK_TRUST_ALL=1` so
  fixture repos never prompt (0013 D7). Tests OF the gate delenv it
  and assert the closed failure (`grip trust add` hint).
- **Offline only.** Sources are `file://` fixture tarballs built in the
  test (`tarfile` module) or tiny local git repos (`git init`). Network
  in e2e is a bug — CI has no credentials and flakes are unacceptable.
  Deno is inherited from PATH (or `GRIPSACK_DENO`); nothing downloads.
- Fixture env repos use the defineEnv contract (0013 D5):
  `conftest.make_env_repo` writes `modules/<name>.ts` files that
  default-export their module value, and generates
  `hosts/<host>.ts` importing them sorted — deterministic module order.
  Adding a module later = write the file + `refresh_host(repo)`;
  undeclaring = `remove_module(repo, name)`.
- The binary path comes from `GRIPSACK_BIN` (default
  `target/debug/grip`); inside docker the gate stage has already
  compiled it — never rebuild from e2e.

## Golden IR corpus (test_golden.py)

The old dual-frontend parity corpus died with the Python frontend
(0013 D1); its replacement is a snapshot corpus: every fixture env in
`fixtures/envs/` is evaluated by the exact Deno invocation the core
uses, with a FIXED inputs file (deterministic facts), and the emitted
envelope (ir + diagnostics + probe_requests) is diffed byte-exact
against `fixtures/golden/<env>.ir.json`.

- Spans are stripped (the only normalization): `span` keys move when a
  fixture is edited without changing the IR's meaning.
- Add a fixture env = drop the directory under `fixtures/envs/`, then
  regenerate: `REGEN_GOLDEN=1 pytest e2e/test_golden.py`. Review the
  snapshot diff like any generated artifact.

## Assertions that matter

- Store paths exist and are immutable between applies (untouched modules
  keep their paths — that IS the reuse test).
- `current` symlink target increments exactly once per apply.
- Rollback restores byte-identical config content, not just paths.
- `tracked-copy` drift: modify the deployed file, re-apply, assert the
  drift report (keep/adopt/restore paths).

## Skips

Tests for unimplemented flows are committed **skipped** with a reason
naming the plan doc (`@pytest.mark.skip(reason="0004: apply")`). Unskip
in the same PR that implements the flow.
