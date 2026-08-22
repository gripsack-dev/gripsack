---
name: gripsack-e2e
description: Write gripsack flow tests — fixture env repos, sandboxed HOME, offline fixtures
---

# Writing e2e flow tests

E2E defends the observable contract of `grip` (plan/0003 §5): apply,
incremental reuse, single-module apply, rollback, drift detection. It runs
the **real binary** against the **real Python frontend** — no mocks of
either side.

## Harness conventions

- Everything under a `tmp_path`: `HOME` is redirected there (plus
  `GRIPSACK_HOME`), so a test can never touch the developer's real
  profile. Assert against the filesystem, not stdout snapshots.
- **Offline only.** Sources are `file://` fixture tarballs built in the
  test (`tarfile` module) or tiny local git repos (`git init`). Network in
  e2e is a bug — CI has no credentials and flakes are unacceptable.
- Fixture env repo layout mirrors plan/0001 §5: `env.toml`, `modules/`,
  `hosts/`. `conftest.py` builds it; tests parameterize the modules.
- The binary path comes from `GRIPSACK_BIN` (default
  `target/release/grip`); inside docker `GRIPSACK_E2E_IN_DOCKER=1` is set
  and the gate stage has already compiled it — never rebuild from e2e.

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
