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
`53c6fac8bc4c4290d66a2f1f5644f4cc8d6063c6`,
`d6d29d7082f69e14e03a262381a776545782961d`,
`ff73b62618359d002040a2d648b87c1d2a6e78bd`,
`123f3bbf9555621d03a0ce989fc02b6abb16b892` and
`01c186fd821eee4a3c077c06ffb7aa53fe38bd7e`. The tracked
behavior-bearing `SOURCE_ROOTS` were clean before and after each
run. The calibration script changed between the first two commits;
the golden/fixture tests changed before the third, the graph/schema/
diagnostic work before the fourth, and selector/exported-command
correspondence before the fifth. Their source fingerprints are not
interchangeable. The ledger cites the **fifth** run for selected
historical A1 cases; three newer E124 owner fixtures change the
tracked e2e source, so no listed report yet binds that source.
Logs carry exact SHA, command, versions and inputs plus raw runner
output and counted result markers. The fifth source fingerprint is
`061e25a86e26178bce09fdbd1ae9df575d006e18e47b5beea9564d4fc89f2919`.
These are local container observations, **not** GitHub CI or native
Mac attestations.

| report | sha256 | observed execution |
|---|---|---|
| `2026-09-24-a1-workspace-53c6fac-e2e.log` | `9206cfdfc213a83819da9743ba3cde023528ae2d28851642b24a20243b4d0f2e` | Real CLI frontend+IR contract/diagnostic suites: 30 passed, 0 failed/skipped; offline sandboxed HOME |
| `2026-09-24-a1-workspace-53c6fac-verus.log` | `082fe2b3f2106ae07e0b1ca24ee410cc0de3b28f856dc4666fa55de9922296c4` | Fresh `cargo verus verify` on the production policy crate: 61 verified, 0 errors; named graph-validation mutant rejected its `required_validation` contract. Other mutants ran, but plan 0048 §6's general attribution hardening remains open |
| `2026-09-24-a1-workspace-d6d29d7-e2e.log` | `eb75090328cfc50dccbb6494c6bdd5717a40d3f5cd6d1a0ecf9658b86c20bb4f` | Earlier source-bound CLI frontend+IR contract/diagnostic suites: 30 passed, 0 failed/skipped |
| `2026-09-24-a1-workspace-d6d29d7-verus.log` | `bca914dc0cb3b6904416aab25da9c7e72275ad11d13845dab39f20353ca68741` | Earlier source-bound fresh Verus 61 verified/0 errors; one attributable graph-validation mutant, not a schema/name-index refinement proof |
| `2026-09-24-a1-workspace-ff73b62-e2e.log` | `f6b3ce0ef0c4b5b418b85c4afd1d00295b67ff369ee6e064d5434b1d3880e206` | Earlier source-bound frontend golden/workspace/diagnostic suites: 35 passed, 0 failed/skipped, including a nine-kind corpus and semantic Bash-env drift mutant |
| `2026-09-24-a1-workspace-ff73b62-verus.log` | `9698ae5182a57810ddedc122097c3facc0df74d065aa1dd9ce475c6cc023d23b` | Earlier source-bound fresh Verus: 61 verified/0 errors; named graph-validation mutant fails its contract, not full schema/name-index refinement |
| `2026-09-24-a1-workspace-123f3bb-corpus.log` | `d59ca5260bd3ea45c36da76a8aa0017ccdfa1190c70c16fb099b6923253ee7be` | Earlier source-bound cross-language corpus: 37 real frontend/CLI plus 3 v3, 3 historical v4 and 5 v5 schema/parser cases; 48 passed, 0 failed/skipped |
| `2026-09-24-a1-workspace-123f3bb-policy.log` | `004bbd63f2b1188cce77c4ca78a4ed4ceb49fe929c6c51665f157d420d405277` | Earlier source-bound direct policy adapter unit suite: 5 passed, including same-role build/runtime target substitutions; no name-index proof |
| `2026-09-24-a1-workspace-123f3bb-architecture.log` | `97a1ffce88192b0af04a4eed1c4cbc2176cefa61700b486e471bbe3b9b9b6586` | Earlier source-bound parsed-TOML architecture self-check: one suite with named member discovery, missing-crate and rustls boundary negatives |
| `2026-09-24-a1-workspace-123f3bb-verus.log` | `cad0d4ad86376d39622c048c7b340d18733872e787a4d26aa7eca1f36612afa2` | Earlier source-bound fresh Verus 61 verified/0 errors, named graph-validation mutant rejected; proof excludes schema/name-index and adapter payload |
| `2026-09-24-a1-workspace-01c186f-corpus.log` | `1c1678be7f728be5017a9f8425c3027a10fe47308b87942e0cd6bebed354bb54` | Earlier source-bound cross-language corpus: 37 real frontend/CLI plus 3 v3, 3 historical v4 and 5 v5 schema/parser cases; 48 passed, 0 failed/skipped |
| `2026-09-24-a1-workspace-01c186f-policy.log` | `6cf259601dfbe8509dd0009ab352ccb260f65a9535a9224558fe3218847553a3` | Earlier source-bound production adapter unit suite: 5 passed, including valid graph, role/target and selector/exported-command substitution cases; no adapter proof |
| `2026-09-24-a1-workspace-01c186f-architecture.log` | `5eeef004595b6cd4917d9965117346496fc79de0d217b97f71d6d9e5fc9d65e0` | Earlier source-bound parsed-TOML architecture self-check: one suite covering named workspace members, missing protected crate and rustls boundary negatives |
| `2026-09-24-a1-workspace-01c186f-verus.log` | `976bd28dccb60a1a6660d78ee79177bbd58bebf2a1de4f6b58d1b889baf9d64d` | Earlier source-bound fresh Verus: 61 verified/0 errors and named graph-validation mutant rejected; selector/name-index adapter unproved |

The records cover selected A1 cases only. They do **not** prove the
schema/name-index refinement, complete A1 proof family, native macOS
behavior, live CI protection or an A1 milestone closure.
