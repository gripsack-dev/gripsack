# verification/reports — archived runner logs (2026-09-24–26)

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
`123f3bbf9555621d03a0ce989fc02b6abb16b892`,
`01c186fd821eee4a3c077c06ffb7aa53fe38bd7e`,
`530ae7db0e4ccc1df7e63e3ddf76597454110e95`,
`8583a4652078a66e2a7d06676a12934ceb8f63d1`,
`6c759a597806ace3f963346701881c7ef7d0d7f3`,
`a754e048360a6146368fd58056f274a4e7f3c1a6`,
`2699d79342da7c70136840828ae3455b0e3eb257`,
`0d7801e0541b32917611dd7ec47455a3a6c59f32`,
`2b7daf8a39dcdae18654b62e9630d21209cd0d0e`,
`ec11c7ce7ffb1ee3ec78c3026c933bf305fabb15`,
`17ed1f0dae5c827d638537fd6efba3623da156f4`,
`90e68acdb63d31e9c56d3161872ed94391aa43a2`,
`17513d2d66c1f99bf40e85d5c9d9e62447c0f88d`,
`d8711b2208b99e73fc9f3ce811405a836f6da977`,
`b4219449fe240b96205a6795de9e28defcaa1983` and
`58388c9539018f8dfa793f9e59895d4e96f9660f` (the last an evidence-only
commit whose `SOURCE_ROOTS` are byte-identical to `b421944`'s). Tracked
behavior-bearing `SOURCE_ROOTS` were clean before and after each
direct runner; changes to calibration, fixtures, schema, graph
adapters, examples and the generated diagnostic registry make
their fingerprints non-interchangeable. Only the focused A1-06
JSON-purity report binds the **eighteenth** source; other A1 row
receipts bind earlier code and cannot support closure for the
changed surfaces. Reports retain commands, versions, inputs, source
identities, raw output and counted markers. The latest source
fingerprint is
`a5352ceff505d1a8bf432d7e482248762d9982de4b19b4f4b8d116f2ceaa8162`;
the preceding destination-ownership revision's fingerprint is
`de296f15fcff42625c58b29d0ee7bd0b2eb09511f9e4aaa1510dd67f5791eca5`.
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
| `2026-09-24-a1-workspace-530ae7d-corpus.log` | `6696ef2415c878bdffdb47d15aee8adc6abccdc6650dc8d92045b181e6f2d89c` | Earlier source-bound corpus: 40 real frontend/CLI cases including E124 owner paths, plus 3 v3, 3 historical v4 and 5 v5 schema/parser cases; 51 passed, 0 failed/skipped |
| `2026-09-24-a1-workspace-530ae7d-owners.log` | `47871eb88b1728c7897210104bde1b5ac71ad3f2f26514d07c0b715fa87d2384` | Earlier source-bound named CLI owners: image B4, environment A2-P and check A2/E1; 3 passed, 14 deselected |
| `2026-09-24-a1-workspace-530ae7d-policy.log` | `d3323a4084868b1cbf9ab1f9eae765c0520a1cf915f7993d2327ee107c7a3950` | Earlier source-bound production adapter suite: 5 passed including role/target and selector/exported-command substitutions; no adapter proof |
| `2026-09-24-a1-workspace-530ae7d-architecture.log` | `d7be0876ccd716d37780547835d1282308192c8e3b4050efe52a501a9b3457c6` | Earlier source-bound parsed-TOML architecture self-check with member, TLS and missing-crate negatives |
| `2026-09-24-a1-workspace-530ae7d-verus.log` | `73564bec9e5a6bef478a5dec5e8accc8a53857c3d22f398b9c124a00279598da` | Earlier source-bound fresh Verus 61 verified/0 errors; named graph-validation mutant rejected, no schema/name-index proof |
| `2026-09-24-a1-workspace-8583a46-corpus.log` | `18c62f721f48df389ed8ecf10f3f9f67b52bd4977af6d41f76f1012617ed18d4` | Earlier source-bound corpus: 45 real frontend/CLI cases plus 3 v3, 3 historical v4 and 5 v5 schema/parser cases; 56 passed, 0 failed/skipped |
| `2026-09-24-a1-workspace-8583a46-examples-composite.log` | `230dbf5000fcbac30860fadb158cc2810543d9f22bcff786e6864b2426d1ef3e` | Earlier source-bound four actual files compiled under TypeScript 7 and five named real admission/alternate-producer cases, 9 checks |
| `2026-09-24-a1-workspace-8583a46-owners.log` | `2d4241801521a8cbe7e6fcb46802596940b7a56b286bf0b3d2074a1c1bb051fc` | Earlier source-bound named CLI image B4, environment A2-P and check A2/E1 cases, 3 passed, 14 deselected |
| `2026-09-24-a1-workspace-8583a46-policy.log` | `e1ac261e36b2073450e3d57cb57ebad4593a77c0ab2bdfdeb3e145f57fde93f8` | Earlier source-bound production adapter suite: 5 passed including role/target and selector/exported-command substitutions; no adapter proof |
| `2026-09-24-a1-workspace-8583a46-architecture.log` | `fa7d5b9cb3ad1e18867f0b9eebe2dd68e0ba0c55e4f1d49e47df9eb60485611e` | Earlier source-bound parsed-TOML architecture self-check; examples participate in SOURCE_ROOTS |
| `2026-09-24-a1-workspace-8583a46-verus.log` | `2b7afbef6374a8e53336db57c0f44302a4d5849394ea29869e83aabe6869dd3d` | Earlier source-bound fresh Verus 61 verified/0 errors and named graph-validation mutant; no A1-10 normalization or adapter bridge proof |
| `2026-09-24-a1-workspace-6c759a5-corpus.log` | `43c2ce642c4f86733924fde0b256221966ff14ce42b698f22ea574d7e8f0af63` | Earlier source: 45 real frontend/CLI cases plus 3 v3, 3 historical v4 and 5 v5 schema/parser cases; 56 passed, 0 failed/skipped |
| `2026-09-24-a1-workspace-6c759a5-policy.log` | `e3da6f7e402906fc7eeac58640533095609228ba8827fdcf9a2edd4f0f7c3efb` | Earlier source: five production policy adapter cases passed; no schema/name-index proof |
| `2026-09-24-a1-workspace-6c759a5-owners.log` | `6294e1a3f0d722fffebda9d82604adf3b1fcbd1e92bf7edb88481b563e6e2a8b` | Earlier source: three named real CLI E124 image B4, environment A2-P and check A2/E1 owner cases passed |
| `2026-09-24-a1-workspace-6c759a5-examples-composite.log` | `4ef77ee979bf1cfee56253910839e9db73ee40a360d0e7b60483d2bdb0eaad7e` | Earlier source: four original examples strictly typechecked under TypeScript 7 plus five named real frontend/CLI admission/alternate-producer cases, 9 checks |
| `2026-09-24-a1-workspace-6c759a5-diagnostics.log` | `cac916755f42d998ef5a8d8aea5a9b48d62b7bba4db4367cfb334b1b9b28f8f9` | Earlier source: generated diagnostic registry fresh (36 core IDs, 5 frontend names), 3 named mutants rejected, Deno 63/63 and 17 named real CLI diagnostic cases; 84 checks, not a classification theorem |
| `2026-09-24-a1-workspace-6c759a5-architecture.log` | `d8c1cfa2a2bc3a8a00d402ac66d6958d50a0fceddd04935c8db4777042163cc8` | Earlier source: one parsed-TOML architecture self-check with dependency, rustls, member-discovery and missing-crate negatives |
| `2026-09-24-a1-workspace-6c759a5-verus.log` | `240cb27f7af898293836964bc3edd84eb4bfde99a21ae5c1d0c68662bd71ef54` | Earlier source: fresh production policy Verus 61 verified/0 errors and five mutants; no adapter or diagnostic proof |
| `2026-09-25-a1-workspace-a754e04-corpus.log` | `a06648e775b79054cc7076764396a6bfe460b2f78d1705a0321f62083c8a288f` | Earlier source: 45 real frontend/CLI plus 3 v3, 3 historical v4 and 5 v5 schema/parser cases; 56 passed, 0 failed/skipped |
| `2026-09-25-a1-workspace-a754e04-policy.log` | `63bc52748e96afedac27b7f54f49e7a5d89142f6b9415860b61896884e872c8e` | Earlier source: six production graph adapter cases including source-kind and target-binding substitutions; no schema/name-index proof |
| `2026-09-25-a1-workspace-a754e04-owners.log` | `4b7bfe3190a28bfcfd81044f663412a95093953eefa9ca358269d4fbc92593c1` | Earlier source: three named real CLI E124 owner cases passed, 14 nonrequired cases deselected |
| `2026-09-25-a1-workspace-a754e04-examples-composite.log` | `d22fefe65424e6808cc46eb220cc4a14b460e3afd9e81b139bf441bde2f3ee8b` | Earlier source: four authored examples typechecked and five named real CLI/frontend admission/alternate-producer cases, 9 checks |
| `2026-09-25-a1-workspace-a754e04-diagnostics.log` | `af1a5dcccec3dc4b0b0e0c2a74d062fd3640733b2634e11ac0b0a8470d113f6a` | Earlier source: generated registry fresh, 3 named mutants, Deno 63/63 and 17 real CLI diagnostics: 84 checks, not a classification theorem |
| `2026-09-25-a1-workspace-a754e04-architecture.log` | `50508d01596c39aa13a688849acec8456a113a9cbd1641e36d013de21adccda5` | Earlier source: one parsed-TOML architecture self-check with dependency, rustls, member-discovery and missing-crate negatives |
| `2026-09-25-a1-workspace-a754e04-verus.log` | `cbfc348d164746077242f2dbf19f11974a44fd9ec8e3ba278bd3bdbd0a197837` | Earlier source: fresh production policy Verus 61 verified/0 errors and five mutants; no schema/name-index/adapter proof |
| `2026-09-25-a1-workspace-2699d79-corpus.log` | `ac609ed7b6b2e110af90794452ad02f2ead9d48e48a04ba4e801f783a54a63ca` | Earlier source: 45 real frontend/CLI plus 3 v3, 3 historical v4 and 5 v5 schema/parser cases; 56 passed, 0 failed/skipped |
| `2026-09-25-a1-workspace-2699d79-target-graph.log` | `00331c54c45eb1a78e10984fd47f9096c3c9950d0140599860cec2a45b6044b3` | Earlier source: six production adapter cases plus two real CLI target cases, 8 checks; no adapter theorem |
| `2026-09-25-a1-workspace-2699d79-owners.log` | `e0a016ddd1d0dc1bfc9f2bcde3109979178a21e3c95de552d6d76d3d3b01c294` | Earlier source: three named real CLI E124 owner cases passed |
| `2026-09-25-a1-workspace-2699d79-examples-composite.log` | `1e751600171de21d22245cd772a37966d813233289c6d6a8fcfdcb4f34dd8def` | Earlier source: four authored examples strictly typechecked and five real CLI/frontend admission/alternate-producer cases, 9 checks |
| `2026-09-25-a1-workspace-2699d79-diagnostics.log` | `83b7c014355e0d94a0ca08877be7d378d172040bc41103652d322b8546c64837` | Earlier source: 36 generated Rust IDs and five frontend names fresh, 3 registry mutants, Deno 63/63 and 17 real CLI diagnostics: 84 checks |
| `2026-09-25-a1-workspace-2699d79-architecture.log` | `4817562e11f5aa4da4432e9d8dd9440bee073cb87ebd773664cf08a591335d3f` | Earlier source: parsed-TOML architecture self-check with member, rustls, dependency and missing-crate negatives |
| `2026-09-25-a1-workspace-2699d79-verus.log` | `5be077ac8497fe1e7ae5183647af2e92db381d0a861d304d680542c398680f67` | Earlier source: fresh production policy Verus 68 verified/0 errors with six mutants; no catalog/name-index/adapter proof |
| `2026-09-25-a1-workspace-0d7801e-catalog-graph.log` | `82336d84353721a4a08f1bf56c8f613dc1010fa3d35834d9c285c2cff5b785d9` | Earlier source: seven adapter cases including the failing-before catalog omission plus two real CLI target cases; 9 passed |
| `2026-09-25-a1-workspace-0d7801e-verus.log` | `f91ce1e9e00517f557b1da0a056366c36280aae09f3f8be795f0862d2c0a6ef3` | Earlier source: policy Verus 68 verified/0 errors with six mutants; catalog mapping unproved |
| `2026-09-25-a1-workspace-2b7daf8-name-index.log` | `cd149daf651ef02e7b4c7b2631c1f4fbb23e93242560e4035c7011663cc0be00` | Earlier source: seven adapter regressions, one exact-name kernel runtime boundary and two real CLI target cases; 10 passed |
| `2026-09-25-a1-workspace-2b7daf8-verus.log` | `bbaf83e3a071594b213225e1dc80d9e3ab1e4586497d40988d45a961f1dbf004` | Earlier source: Verus 72 verified/0 errors with seven policy mutants; full schema/name-index mapping remains unproved |
| `2026-09-25-a1-workspace-ec11c7c-diagnostics.log` | `9fa928542cfe1d57974824b3e7222180d410363c5c8fefffec5946299308d044` | Earlier source: generator freshness, three named registry mutants, Deno 63/63 and 18 real terminal/JSON CLI diagnostics including failing-before unallocated E999 rejection; 85 checks, no semantic classification theorem |
| `2026-09-25-a1-workspace-17ed1f0-source-site.log` | `b1f9a4524338d787d0671372889251f14eec01b2cdedec180671989b12841ed8` | Earlier source: eight policy adapter cases including failing-before provenance substitution plus 23 real CLI workspace cases; 31 checks, no full source-walk/schema refinement proof |
| `2026-09-25-a1-workspace-17ed1f0-verus.log` | `dc8d3f35e43ed1956b31f8f80e6a0f09175979c5514d9d2a4f2e5f20f8fc65a2` | Earlier source: fresh policy Verus 72 verified/0 errors with seven attributable mutants; TypeScript and TLC compose images cached, no full source-walk/schema refinement theorem |
| `2026-09-26-a1-workspace-90e68ac-span-owners.log` | `1d9da4a0bc6f79aebdb678c5472561f8a16ebe5c11bb3381bbdcc59cbb757c10` | Earlier source: failing-before labeled E129 span admission (5 Rust), five first-declared E124 owner/precedence cases plus the no-snippet malformed-coordinate case (6 real CLI), and the same-source Deno gate layer incl. the compile-time interpreter pin; 74 checks, no classification/normalization theorem |
| `2026-09-26-a1-workspace-17513d2-destination.log` | `c89573d55367642b7baf5fd85d32f67384a37ef2c479a7b989f80f85a0a65f11` | Earlier source: failing-before E102 destination-escape admission (6 Rust) plus ten real CLI destination/owner/coordinate cases and the same-source Deno gate layer (64 tests + tsc examples); 81 checks, tree origin and proof targets open |
| `2026-09-26-a1-workspace-d8711b2-dest-collision.log` | `4c7eaa90a6ff2b8300b9883c28a561b754c7b3b9e2cd630ac385fe2051a55240` | Earlier source: failing-before case-folded duplicate-destination ownership (E111) across profiles and policies labeling every declaration, plus the equal-basename coexistence positive; 3 Rust + 2 real CLI checks, per-block marker grammar open |
| `2026-09-26-a1-workspace-b421944-json-purity.log` | `cea083cb82f29507cdae19b4b1d13956a0df7ced17c0953bcd5d3b97274bd61a` | Earlier source: failing-before core-side E111 tracing line polluted check --json stdout (Extra data); console logs now go to stderr and the parity helper asserts help text on both surfaces; 9 real CLI checks, fresh full chain e2e 299/299 |
| `2026-09-26-a1-workspace-58388c9-a1-08-register.log` | `1293117cc533a9949f8df4aeeb3a1c6848dd6fba66a3110be908df187ccaa272` | Current source (identical SOURCE_ROOTS to b421944): A1-08 live registration — shared command grammar over nine kinds, forged-field rejection, ambient E128, inert-schedule/task-prereq owners, first-declared precedence; 14/14 real CLI cases, identity/kernel obligations open |
The records cover selected A1 cases only. They do **not** prove the
schema/name-index refinement, complete A1 proof family, native macOS
behavior, live CI protection or an A1 milestone closure.

## M0 §1.3 host admission — local evidence, not release closure

The original edition-5 handover checksums pass for all eight files,
and the live delivery ledger contains all **178** IDs. At source
commit `681e87b439cd979978a12dbefab2eccb18989944` the tracked
`SOURCE_ROOTS` fingerprint is
`11419c4e1d532fea98b8b2e687f0fed60d964d9388413f4111dfd9a53971968c`.
The focused runner checked clean roots before and after; the separate
five-gate run was **pre-commit worktree** observation with inferred
source equivalence, not an exact-commit CI attestation.

| report | sha256 | observed execution |
|---|---|---|
| `2026-09-26-m0-host-681e87b.log` | `65a710e52fd7d343b96aa3d4c34baa7138b73ed3c15129ff8e585f880680636c` | Committed source: E132 typed-host unit, direct lockfile roundtrip and five real CLI cases including the three original failing-before host/adopt witnesses; 7/7, sandboxed HOME, no outside file writes |
| `2026-09-26-m0-host-precommit-five-gates.log` | `9249ecbb2e1a0a39bb5d0360bd93d90391641c708100a0e89e72e072e281e5ce` | Pre-commit worktree: fresh Rust fmt/clippy/tests, real CLI e2e 304/304, fresh Verus 72/0 with seven named mutants; cached TypeScript/TLC image layers — not protected CI or Mac evidence |

This binds only plan/0048 §1.3's selected Linux behavior. All 178
original handover lane/case/evidence-kind inventories were registered
later without verifying any new row. H0 source-bound review, checker
kind/proof enforcement, other NEXT leaves, protected CI, native Mac/VM
and public release remain open.

## M0 §1.4 evaluator bounds and adopt trust — local, not release closure

| report | sha256 | observed execution |
|---|---|---|
| `2026-09-26-m0-boundary-0b92905.log` | `2048aa748eb5f867ec0412aa5fea74b3007468711be7062e0b4606ef048dca71` | Exact committed source `0b92905`, clean tracked source roots, fingerprint `a53a49a4d505774fa43f1882515e4ceeb15890c3542c635deaaad1575c862b6b`: two Rust regressions and ten sandboxed real CLI cases, 12/12. Two hostile pinned-runtime stubs exercise only the 16 MiB stdout/stderr supervisor; real Deno covers ordinary probe, host and trusted adopt flows |
| `2026-09-26-m0-supervision-precommit-five-gates.log` | `b47fcb9a803481d70454119bea308fed210f34ce6241485fdff4d30a84d77192` | Pre-commit dirty worktree: fresh Rust fmt/clippy/tests, real e2e 307/307 and fresh Verus 72/0 with seven named mutants; `ts-test`/`model` passed without fresh RUN output ([INFERENCE] cached), not source-bound protected CI |

The original 178 IDs now have registered lane/case/evidence-kind
inventories, but source-bound H0 inventory review, stronger proof/kind
checks, other NEXT leaves, required formal campaigns, native Mac/VM
and actual release gates remain open.

## H0 edition-5 inventory — source-bound structure, not semantic closure

| report | sha256 | observed execution |
|---|---|---|
| `2026-09-26-h0-inventory-953724a.log` | `320e84504d7ab77f4f4b8d8ccdfc62e435cc878b5207317d9d4ed204d0905b9d` | Committed source `953724a`, fingerprint `68ef13d7676043c886626237cff226afdb9d0529f80b4f449fb83f13787e514d`: original eight checksums pass and 178/178 IDs and five immutable index fields match the live ledger; all lanes/cases/kinds are nonempty and no ID is verified. The pre-record ledger input digest is in the report; inserting this receipt changes the ledger bytes |
| `2026-09-26-h0-kinds-56b1581.log` | `d4feaa90377df2bb2e757dfb437c51334071ff1c89578fef713f24dd14e78964` | Committed source `56b1581`, fingerprint `a145624cac62ebd3a39741ccc21042d6c8d20dc6086a0454bdc726374a40fe38`: synthetic H0/A0 and review-only positives passed, **23/23** named negative checker mutants rejected (wrong kind, runner masquerading as formal, missing proof names/floors/report bytes, skipped lanes and stale source); live H0 closure correctly fails with missing proof catalogs and pending H0/A0 rows. Calibration is not an actual proof or independent semantic review |

Both reports cover the inventory/checker structure, not independent
semantic review of all 178 acceptance cases. H0-02 stays
`in_progress`: the 39 formal rows need actual named proof catalogs
and count floors, and global/native/VM gates require their own
reports. The synthetic checker fixture cannot close H0 or A0.

## M0 §1.1 comma-grant rejection — committed local behavior, no release

| report | sha256 | observed execution |
|---|---|---|
| `2026-09-26-m0-grants-953724a.log` | `861c6dce2000c764e77a54244ddbafa3cd812c9dbcd4b18d189114c61a61fbc7` | Exact committed `953724a` source with fingerprint `68ef13d7676043c886626237cff226afdb9d0529f80b4f449fb83f13787e514d` and clean tracked source roots: one Rust grant-construction unit, five real CLI pin/repo/home/valid-control cases and 12 freshly executed Deno driver/pin cases, 18/18. Original pre-fix comma pin read an outside canary and returned success; after E133 both terminal and JSON fail before spawn |
| `2026-09-26-m0-grants-local-five-gates.log` | `8efaa16213c359407583dd8fcdadc66d84277771d7021db13e52e5c0ec7c5a82` | Local five Compose services pass on the `953724a` source tree: fresh Rust fmt/clippy/tests (38 generated core codes), real CLI e2e 311/311, fresh Verus 72/0 plus seven named mutants. TypeScript and TLC RUN layers explicitly CACHED. No runner-time source-clean assertion: source equivalence is [INFERENCE], not exact-commit protected CI |

Protected PR `audit` passed with rustls 0.23.45, while the external
TypeScript example remains failed on an unpinned Pixi ripgrep tree
digest. M0 §1.2, R1/R5, native Mac/VM and all other release-blocking
NEXT/proof work are still open.

## M0 §1.2 repo-env/credential/TLS boundary — committed local packet

`1c693efbfc8335f52888a77efa68ba266132b1ed` has clean tracked
`SOURCE_ROOTS` before/after the direct runner and fingerprint
`9a82d56b48bef69d8a57c831a3efa16638123279150689d277ecdd8a81447825`.

| report | sha256 | observed execution |
|---|---|---|
| `2026-09-26-m0-env-1c693ef.log` | `ac2b231938c22fba1ca1abd192b5453edf4045b13e0d655681f74c9140ebf3d1` | Two direct Rust config/HTTP admission tests and ten sandboxed real CLI build-shell, structured PATH, plugin, Deno-isolation, proxy/CA, local TLS, wrong-/same-host redirect, E400 terminal/JSON and HTTP-cleartext cases passed **12/12**, zero skipped. Direct TLC ran four credential-routing cfgs: clean base plus three named base-authority/redirect/repo-audience counterexamples, **4/4**. The real pre-fix PATH detector shim ran, and the old GH_HOST rebind let a dummy token reach an HTTP fixture; neither occurs on this source |
| `2026-09-26-m0-env-local-five-gates.log` | `fd4edb5f0f720971e5e77b2fa0b01842ef189b9d24e438287715fc70538c745d` | Local final-source Docker observations: fresh Rust fmt/clippy/tests, full **319/319** real e2e, uncached direct Deno **64/64** plus examples typecheck, direct full TLC in the committed-source report, fresh Verus **72/0** with seven named mutants. Tracked SOURCE_ROOTS match `1c693ef`; the four compose outputs lack runner-time commit markers, so full-gate exact-head attribution is **[INFERENCE]**, not protected CI or a native Mac proof |
| `2026-09-26-m0-env-macos-ci-1c693ef.log` | `31e7a23e1377a146df0ef8fda2bef473a30cd831162325a2d213736dec71f3c8` | GitHub [e2e-macos job 108365100726](https://github.com/gripsack-dev/gripsack/actions/runs/36227812302/job/108365100726) checked out exact `1c693ef` and built real grip on macOS **14.8.9 arm64** with pinned Rust 1.98.0/Deno 2.9.6/Python 3.12.10. Full native flow **319/319 passed**, including the new scoped env/TLS/redirect cases. This is neither nested Mac-VM/launchd scheduling qualification nor branch-protection enforcement |

The same implementation source passed the local Docker Rust
fmt/clippy/tests gate and the full **319/319** real CLI/Deno e2e suite;
an uncached direct TypeScript/frontend run checked **64/64** Deno tests
plus strict example typecheck; final-source Verus checked **72**
obligations with zero errors and seven named mutants rejected. The
earlier 316-case/72-obligation snapshots are historical, not counted
as final-source evidence. Manually dispatched CI run
[`36227812302`](https://github.com/gripsack-dev/gripsack/actions/runs/36227812302)
checked out exact `1c693ef`: its Linux `test`, native arm64 Mac
`e2e-macos`, `audit`, `fuzz` and `docs` jobs all passed. The later
checker-source `56b1581` differs and needs its own CI; dispatch
does not enforce branch protection or qualify Mac-VM. The receipt is
a selected plan/0048 control, not H0/G-05 verification, M-V7 proof or
a release.
