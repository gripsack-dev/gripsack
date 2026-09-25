# verification/reports — archived runner logs (2026-09-24)

Unmodified copies of the gate logs captured on 2026-09-24 in `/tmp/gripsack-baseline/`
(pristine-worktree baseline runs) and `/tmp/gripsack-a0/` (A0-tree runs), archived
2026-09-24. Every copy is byte-identical to its source (SHA-256 below, verified after
copy). `2026-09-24-baseline-verify.log` contains carriage-return progress bytes; the
archive preserves them exactly.

## Binding statement (honest limits)

- File mtimes (2026-09-24 08:14–08:56 BST = 07:14–07:56 UTC) are consistent with the
  run windows recorded in plan/0049-handover-bundle-import.md (baseline gates
  07:14–07:35 UTC at pristine `176eaecd85a95b4f29ed49293a889c7bf16cc8c8`; A0-tree gates
  07:36–07:56 UTC).
- The source-commit attribution itself is attested only by plan/0049 prose. The exact
  source revision and dirty-tree identity of each run is **not independently
  recoverable** from these logs (they carry no commit marker), and several logs show
  BuildKit **CACHED** test layers rather than fresh execution (see observations).
- Therefore these logs are retained as historical runner observations. They do NOT
  satisfy the hardened runner-evidence contract (repo-local report + SHA-256 +
  executed/passed/failed/skipped counts + observed marker + tool versions/inputs +
  exact source commit/dirty identity) and MUST NOT be cited as `verified` evidence
  without a fresh source-bound rerun.

## Files and observations

| file | sha256 | source | bytes | observation |
|---|---|---|---|---|
| 2026-09-24-baseline-test.log | 4dbd3196865f9b4ce32a64db17e415923b291dda072ad3dc0ec0a7aea36f6ffd | /tmp/gripsack-baseline/test.log | 3941 | ends `gate passed: fmt + clippy -D warnings + cargo test`, BUT the test layer `#11 [test 1/2] RUN cargo fmt/clippy/test` is `#11 CACHED` — no fresh test execution in this capture |
| 2026-09-24-baseline-ts-test.log | 132a195b22d36bb07cc231df457b13de8c2f80a22159e8f3018189c3a526f2c4 | /tmp/gripsack-baseline/ts-test.log | 2348 | ends `gate passed: typescript frontend tests (deno test)`, BUT test layer `#11 RUN cd typescript && deno install && deno task test` is `#11 CACHED` |
| 2026-09-24-baseline-e2e.log | 5afb981cfc5f4e871bcb60465376f0c0415f3b9d62176980e4ad3cb9d81c6c23 | /tmp/gripsack-baseline/e2e.log | 7921 | pytest executed in container: `245 passed in 884.23s (0:14:44)` |
| 2026-09-24-baseline-model.log | 7e2ee79aaad6bbe70c49a4558b735e7506d89a24e650719756a15da3ffb19e29 | /tmp/gripsack-baseline/model.log | 6144 | TLC executed: 60 config checks listed (46 `cfg/` + 7 ProcessSupervision + 7 UpdatePublication), `#14 DONE 49.2s`, `gate passed: model checks` |
| 2026-09-24-baseline-verify.log | 5017000a14824576bae65308e489893aa8d7fade7a7a52e02bf96ec7ce36144f | /tmp/gripsack-baseline/verify.log | 35459 | Verus executed: `56 verified, 0 errors` + 4 seeded mutants rejected, `#20 DONE 139.3s`, `gate passed` (contains CR progress bytes, preserved) |
| 2026-09-24-a0-test.log | f08bf75f8acc4e44b723b7006665601db34f05f850f8a517d2aa462fc38122b1 | /tmp/gripsack-a0/test.log | 61508 | cargo fmt/clippy/test executed (`#18 DONE 69.3s`): per-crate results incl. gripsack-fetch `61 passed; 0 failed` (13 bottle-selection cases), all suites 0 failed; `gate passed` |
| 2026-09-24-a0-ts-test.log | c5fead981c51c65903e85c35e379fecd9bb8ecd613d35a7c74c8ec1bc49e53fe | /tmp/gripsack-a0/ts-test.log | 2226 | ends `gate passed: typescript frontend tests (deno test)`, BUT test layer is `#11 CACHED` |
| 2026-09-24-a0-model.log | ba9090790aabc3403df961890c37239657aa768f1320dc8c855b70126a470e11 | /tmp/gripsack-a0/model.log | 2809 | ends `gate passed: model checks`, BUT TLC layer `#14 RUN sh scripts/check_models.sh` is `#14 CACHED` |
| 2026-09-24-a0-verify.log | 82b51d9000faf8623237e3b018d9e3ccc11b98b734c66df9d0fa02bc3a80cc07 | /tmp/gripsack-a0/verify.log | 4463 | Verus executed: `56 verified, 0 errors` + 4 seeded mutants rejected, `#20 DONE 144.8s`, `gate passed` |

Note: the earlier G-03 ledger record's handwritten `obligations.checked: 50` does not
match either verify log; both show `56 verified, 0 errors`. The logs are the
authoritative observation.

## Source-stamped A1 worktree runs

Runner reports exist for source commits
`53c6fac8bc4c4290d66a2f1f5644f4cc8d6063c6` and
`d6d29d7082f69e14e03a262381a776545782961d`. The tracked
behavior-bearing `SOURCE_ROOTS` were clean at each run. The later
commit changes the delivery calibration script, so the first
fingerprint cannot bind evidence to the later code. The delivery
ledger cites the **second** run. Logs carry the exact SHA, command,
versions and inputs plus raw runner output and counted result markers.
They are local container observations, **not** GitHub CI or Mac
attestations.

| report | sha256 | observed execution |
|---|---|---|
| `2026-09-24-a1-workspace-53c6fac-e2e.log` | `9206cfdfc213a83819da9743ba3cde023528ae2d28851642b24a20243b4d0f2e` | Real CLI frontend+IR contract/diagnostic suites: 30 passed, 0 failed/skipped; offline sandboxed HOME |
| `2026-09-24-a1-workspace-53c6fac-verus.log` | `082fe2b3f2106ae07e0b1ca24ee410cc0de3b28f856dc4666fa55de9922296c4` | Fresh `cargo verus verify` on the production policy crate: 61 verified, 0 errors; named graph-validation mutant rejected its `required_validation` contract. Other mutants ran, but plan 0048 §6's general attribution hardening remains open |
| `2026-09-24-a1-workspace-d6d29d7-e2e.log` | `eb75090328cfc50dccbb6494c6bdd5717a40d3f5cd6d1a0ecf9658b86c20bb4f` | Current source-bound CLI frontend+IR contract/diagnostic suites: 30 passed, 0 failed/skipped |
| `2026-09-24-a1-workspace-d6d29d7-verus.log` | `bca914dc0cb3b6904416aab25da9c7e72275ad11d13845dab39f20353ca68741` | Current source-bound fresh Verus 61 verified/0 errors; one attributable graph-validation mutant, not a schema/name-index refinement proof |

The records cover selected A1 cases only. They do **not** prove the
schema/name-index refinement, complete A1 proof family, native macOS
behavior, live CI protection or an A1 milestone closure.
