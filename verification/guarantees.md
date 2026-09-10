# Guarantee ledger

One row per guarantee: what is promised, where it is enforced, what is
trusted, and what evidence backs it. Statuses: `proposed`,
`implemented-unverified`, `checked` (machine proof), `blocked`,
`superseded`. Populated with evidence, not intentions (hardening
handoff §8). Proof work is on the roadmap; today's evidence is
regression tests, explorers, models and e2e.
| ID | Claim (with exclusions) | Enforcement | Admission boundary | Trusted components | Coverage | Bridges | Calibration | Status |
|---|---|---|---|---|---|---|---|---|
| CLASSIFY-001 | The commit classifier decides by exact transaction identity only: Committed ⟺ current == target; Uncommitted ⟺ fresh machine with no current, or current == previous ≠ target (target precedence explicit); Ambiguous otherwise. Numeric ordering and Apply/Rollback labels never decide. Exclusion: classification is only as good as the marker parse (MARKER-PARSE-001) and the current-pointer read. | Verus proof over `classify`, `gripsack-policy/src/lib.rs`; production reconcile, both explorers and the repeated-crash model call the same function | marker admission (F2) establishes the facts; no hidden preconditions — total function over all `RecoveryFacts` | Verus 0.2026.09.06.8dea4a2, Z3 4.16.0, Rust 1.98.0, vstd =0.0.0-2026-09-06-0133 | universal over the admitted input domain | `journal::tests`, both explorers, torn-marker e2e | seeded ambiguity→committed mutant fails its named postcondition in `scripts/check_verus.sh` | **checked** |
| OWNERSHIP-001 | Copy/link authority: Fresh ⟺ nothing live; TakeOver ⟺ consent && something live; Satisfied ⟺ live == desired without consent; Update ⟺ live == the last managed write (never preserved drift) and ≠ desired; Preserve otherwise. Links: Refuse ⟺ foreign, unrecorded, unconsented; TakeOver ⟺ consent over foreign+unrecorded; Link otherwise. Exclusion: decisions are over admitted identities; observation and effects live in exec. | Verus proofs over `plan_copy`/`plan_link`, `gripsack-policy/src/ownership.rs`; the lineage explorer and all planning call the same functions | identities arrive as typed values from exec observations; no hidden preconditions — total over all inputs | same pinned Verus/Z3/Rust toolchain as CLASSIFY-001 | universal over the input domain | lineage explorer (materialized filesystems), ops VM harness, ownership e2e | the authority table is the contract; a drift-promotion mutant would fail `Update`'s biconditional | **checked** |
| MARKER-PARSE-001 | A run marker missing `previous_generation` never reaches the classifier; explicit `null` remains the legitimate fresh-machine form. | manual `Deserialize` for `RunMarker`, `journal/marker.rs` | `run_marker()` reader; torn/corrupt markers fail closed with the journal retained | serde_json | all marker bytes | parser tests (missing/null/value/duplicate/overflow/wrong-type); torn-marker e2e asserts parser rejection | pre-fix: missing field parsed as `null` (the F2 defect) — parser test fails there | implemented-unverified |
| GC-RECOVERY-001 | `gc` never deletes objects an unfinished recovery needs: pending recovery state blocks collection (dry-run included, nothing deleted), the current generation is never pruned, and the deletion set is exactly candidates-minus-roots and monotone in the roots. Exclusions: root COMPLETENESS at production time (a manifest missing a dependency) is a separate obligation; a precise classify-aware admission (a committed run's entries need no priors) is deferred. | Verus proofs over `admit_gc`/`plan_prune`/`plan_delete` + `lemma_delete_monotone`, `gripsack-policy/src/retention.rs`; `journal::pending_recovery` + the session-bound collector in `gripsack-exec/src/gc.rs` | `LifecycleSession` held; unreadable journal/quarantine/manifests and non-UTF-8 inventory fail closed before any plan | cap-std Dir listings, UTF-8 path admission | universal over admitted inventories | `gc::tests` (crash-window refusal both modes, quarantine, post-reconcile collection), e2e crash → gc → apply → gc | pre-0.40: gc collected journal-only priors; a `delete`-includes-roots mutant fails `plan_delete`'s extensional contract | **checked** |

## Limits that stay visible

- The lifecycle lock serializes cooperating gripsack processes for one
  home. It excludes neither editors nor arbitrary external writers.
- A pinned parent capability prevents pathname-redirection races; it is
  not an atomic compare-and-swap of bytes/mode against all writers. The
  observation-to-write window is documented on the safety page.
- Hash equality assumes SHA-256 collision resistance for content
  identity; encoding injectivity (RECOVERY-IDENTITY-001) is separate
  and does not depend on it.
- Process-crash tests and power-loss models are different evidence:
  kernel caches surviving `kill -9` say nothing about physical storage
  failure; the TLA+/explorer power-loss lanes cover the reordering
  envelope, not the hardware.
- File/directory sync and rename semantics are trusted OS/filesystem
  contracts, qualified to the platforms CI exercises.
- The current-pointer switch is a logical generation commit; outside
  readers do not receive an atomic multi-file snapshot.
- Activation hooks can be replayed after a crash; neither
  exactly-once effects nor eventual hook success are promised.
- Verified or not, policy functions do not prove Deno, TLS, the
  compiler, the solver or storage hardware.
