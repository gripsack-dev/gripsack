# Guarantee ledger

One row per guarantee: what is promised, where it is enforced, what is
trusted, and what evidence backs it. Statuses: `proposed`,
`implemented-unverified`, `checked` (machine proof), `blocked`,
`superseded`. Populated with evidence, not intentions (hardening
handoff §8). Proof work is on the roadmap; today's evidence is
regression tests, explorers, models and e2e.

| ID | Claim (with exclusions) | Enforcement | Admission boundary | Trusted components | Coverage | Bridges | Calibration | Status |
|---|---|---|---|---|---|---|---|---|
| RECOVERY-IDENTITY-001 | Journal entry identities are variant-disjoint: a symlink target can never spell a file identity or a removal intent, so a post-crash object is never misclassified as the mutation. Exclusion: hash equality still assumes SHA-256 collision resistance for CONTENT identity — encoding injectivity is the property here. | `IntendedSerde` tagged wire + `Entry::from_wire` admission, `gripsack-store/src/journal/mod.rs`; typed `decide_from`, `journal/recover.rs` | `read_uncommitted` quarantines legacy/unsupported/malformed entries with reasons; reconcile refuses over a non-empty quarantine | serde_json, cap-std Dir reads | all admitted entries; legacy (pre-0.40) entries fail closed by construction | `journal::tests`: cross-variant inequality, sentinel-symlink witness through `reconcile`, round trips; explorers drive the typed `decide_from` | pre-0.40 code: sentinel-symlink witness restores (deletes) a user symlink — the new test fails there | implemented-unverified |
| MARKER-PARSE-001 | A run marker missing `previous_generation` never reaches the classifier; explicit `null` remains the legitimate fresh-machine form. | manual `Deserialize` for `RunMarker`, `journal/marker.rs` | `run_marker()` reader; torn/corrupt markers fail closed with the journal retained | serde_json | all marker bytes | parser tests (missing/null/value/duplicate/overflow/wrong-type); torn-marker e2e asserts parser rejection | pre-fix: missing field parsed as `null` (the F2 defect) — parser test fails there | implemented-unverified |
| GC-RECOVERY-001 | `gc` never deletes objects an unfinished recovery needs: any run marker, journal entry or quarantined entry blocks collection, dry-run included, with nothing deleted. Exclusion: a PRECISE policy (a committed run's entries need no priors) is deferred; the conservative rule blocks slightly more than necessary. | `journal::pending_recovery` + admission in `gripsack-exec/src/gc.rs` | `LifecycleSession` held; unreadable journal/quarantine state fails closed | cap-std Dir listings | all journal states gc can observe | `gc::tests`: crash-window refusal (both modes), quarantine refusal, post-reconcile collection; e2e crash → gc → apply → gc | pre-fix: gc collects the journal-only prior blob and recovery loses the bytes — e2e fails there | implemented-unverified |
| LOCK-SESSION-001 | The lifecycle lock cannot be skipped or mis-bound by library consumers: `gc`, `rollback_generation` and `verify_store` require a `LifecycleSession`, which owns the flock and the home it covers. Exclusion: store-side `reconcile` keeps a prose contract (it sits below exec); editors and shell scripts are outside the lock by design. | `gripsack-exec/src/util.rs` `LifecycleSession`; mutation signatures take `&LifecycleSession` | `LifecycleSession::acquire` is the only constructor | fs flock semantics | compile time (type-level) | `gc::tests::a_session_for_another_home_authorizes_nothing_here`; CLI commands acquire sessions | pre-fix: `gc(home, …)` ran without any lock proof | implemented-unverified |

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
