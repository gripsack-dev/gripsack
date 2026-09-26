# 0049 — Handover bundle import (edition 5) and H0 reconciliation

Status: H0 in progress (H0/A0 closure claims retracted 2026-09-24 — see Record) · Owner: implementation agent · Date: 2026-09-24

The six-document implementation handover (bundle edition 5, 23 September
2026, checksums verified) is the governing roadmap for this line of
work. `verification/delivery.json` is its registered inventory: 178
mandatory delivery IDs across five closure scopes (foundation,
foundation extensions, tasks/schedules, semantic change, artifact
sharing). The index seeds the ledger; the specifications' prose remains
normative and cannot be narrowed by the ledger.

## H0-01 — live-repository reconciliation

- **Live revision:** `176eaecd85a95b4f29ed49293a889c7bf16cc8c8`
  (= the handover's research baseline) on `main`, plus local
  uncommitted work: an `AGENTS.md` section and `plan/STATUS.md` row
  adding the plan-0048 implementation contract, and the untracked
  `plan/0048-review-response-0.42.0.md`. All preserved, none reset.
- **Contract read:** `AGENTS.md`, `plan/STATUS.md`,
  `verification/guarantees.md`, and the plans named by Epic A §2
  (0001, 0002, 0003, 0013, 0035, 0036, 0039, 0042, 0046, 0047).
- **Relationship to plan 0048 (ruling):** plan 0048 is binding
  repo-local scope for any public release of the current line: its
  NEXT (M0–M2) items and release checklist gate publication. The
  bundle's milestones proceed independently of 0048 implementation
  order, and every public release cut under this program must pass
  0048 §9 in full. Neither program authorizes weakening the other.
  (Owner may amend; recorded here because the bundle says preserve
  local work, and 0048 says an agent cannot downgrade its NEXT.)
- **Toolchain/platform inventory (this workstation):**
  - Docker 29.7.2 + Compose v5.5.0, linux/amd64 — all five container
    gates runnable, including `verify` (Verus is amd64-only).
  - systemd user/system manager present (WSL2) — E's Linux scheduler
    lane is locally qualifiable.
  - No macOS hardware, no virtualization-capable Mac runner — every
    native macOS / launchd / Mac-VM lane is **blocked**, recorded as
    blocked, never substituted (bundle rule: missing Mac = blocked).
  - Host `go`/`deno` absent — irrelevant to gates (container-provided).
- **Already-landed mappings:** the bundle lands no implementation by
  itself; the repo's existing fetch/store/exec/policy work is
  prerequisite scaffolding, not verified bundle rows. All 178 rows
  start `pending` in the ledger.

### Unmodified baseline gates (176eaec, pristine worktree)

Run in a detached `git worktree` so concurrent implementation cannot
contaminate the record (gates bake verification into image builds from
the live build context).
Run of 2026-09-24 (07:14–07:35 UTC); logs originally in `/tmp/gripsack-baseline/`,
now archived byte-exact (SHA-256 manifest) in `verification/reports/` as
`2026-09-24-baseline-*.log` (see `verification/reports/README.md`). Honest
limitation recorded at archival: the `test` and `ts-test` logs show the test
layers as BuildKit `#CACHED` (content-keyed reuse, no fresh execution in that
capture), and none of the logs carries a commit marker, so the archived files
are historical observations, not source-bound runner evidence.

| Gate | Result | Wall |
|---|---|---|
| test (fmt + clippy -D warnings + cargo test) | PASS | ~2s (content-keyed image cache; layers derive from the pre-edit context snapshot) |
| ts-test (deno) | PASS | ~1s (cached) |
| e2e (real binary + frontend, offline fixtures) | PASS | 14m51s |
| model (TLC, positive + negative cfgs) | PASS | 53s |
| verify (Verus proofs + mutant calibration) | PASS | 5m49s |

No pre-existing failures at the baseline: any later gate failure is
attributable to new work, not the inherited state.

## H0-02 — delivery and support inventory

`verification/delivery.json` (format `gripsack-delivery-ledger` v2)
registers every ID with owner milestone, closure scope, source
document, verbatim deliverable/evidence, and honest status. All
**178/178** original IDs now have registered original-spec platform/
capability lanes, conjunctive case/proof inventories and explicit
runner/formal/review evidence kinds; later A/B/E/C/D rows still remain
pending. Distinct `macos-vm`, native arm64 Mac/launchd, real registry,
isolated recipient, formal, fuzz and OS-timing lanes prohibit a Linux
or generic hosted Mac result from inheriting an unrun capability.
Source-bound H0 inventory review/runner evidence, stronger checker
kind/obligation enforcement and all required H0/A0/global lane results
remain open; H0-02 is not verified. A1 (A1-07) implements
`scripts/check_delivery.py`: inventory validation, scoped closure
checking, and the required negative calibration; H0/A0 evidence must
survive that checker or be repaired, not grandfathered. On 2026-09-24 the
checker was hardened (runner evidence must be a repo-local report with
SHA-256 and executed/passed/failed/skipped counts; closure binds its
declared source SHA to the actual checkout). The earlier Markdown-only
H0/A0 evidence was reopened because no gate log independently records
the tested commit or dirty patch identity:
H0-01/A0-01 are `implemented_unverified`; H0-02 remains `in_progress`.
At reopening 149 of 178 rows had null inventories; registering the
29 A2/A2-P/B1/B2/C0/D0 rows, the remaining 120 A/B/E/C/D rows and the
four A1 placeholder proof/case lists reduced unregistered inventories
to **zero** on 2026-09-26. Explicit evidence kinds are now registered
on every ID too. None of those registrations claims native, VM,
registry, parser/fuzz or proof execution. Prior invalid claims remain
under `historical_claims`.

## E0-02 — OS scheduling qualification (systemd lane)

Qualification on this workstation (WSL2, linux/amd64): systemd 255
(`255.4-1ubuntu8.17`), user manager `running`, `Linger=no`, session
bus present. `systemd-analyze calendar daily` normalizes to
`*-*-* 00:00:00` with host-local timezone (BST) as a runtime fact.
Bounded fixture journey `gripsack-e0-qual.{timer,service}` (own labels,
`Persistent=false`, `AccuracySec=1s`, `Type=oneshot`, `Restart=no`,
explicit `Unit=` binding): installed via `daemon-reload` + start,
fired exactly at the `OnCalendar` minute, then stopped, removed and
reloaded — zero residue (no timers, no files, no failed units). The
launchd/macOS lane stays **blocked** (no Mac); recorded in the ledger
with lane-scoped status, never inferred from Linux results.


## A0-01 — pure OS/architecture/version-aware bottle selection

The lexical `bottle_key` (reverse BTreeMap iteration) selected Linux
bottles on Intel Macs and inferred macOS chronology from tag spelling.
It was replaced by a pure policy over injected facts:

- `HostPlatform { os, arch, macos_version }` is captured once per
  `FetchContext`; the policy itself never reads the environment.
  On macOS `/usr/bin/sw_vers -productVersion` runs through the existing
  `gripsack-process` supervisor with a three-second deadline and
  bounded output, not an unbounded `Command::output()`. Missing, noisy
  or malformed facts refuse selection rather than guessing. This
  macOS probe remains unqualified on this Linux workstation.
- Tag grammar recognized structurally (`arm64_linux`, `x86_64_linux`,
  `arm64_<codename>`, `<codename>`, `all`); chronology lives only in
  the codename table (`high_sierra`…`tahoe`), never in tag ordering.
- macOS selects the greatest codename whose minimum version the host
  satisfies; older bottles run on newer macOS, never the reverse.
  `all` is a last resort under that validated policy.
- Refusals name the host facts and each available tag's rejection
  reason (wrong platform, requires macOS ≥ N, unrecognized).
- Transport unchanged: `resolve_brew` still returns the formula's URL
  and sha256; locked pins bypass selection entirely.

Acceptance cases (conjunctive, in `bottle.rs` tests): Intel-Mac
lexical counterexample (`x86_64_linux` present, `sonoma` chosen);
Apple-Silicon chronology (`arm64_sonoma` over `arm64_ventura`);
incompatible/Linux-only tags refused; older-macOS refusal names the
required version; unknown tags reported; `all` last-resort policy;
unsupported architecture fails clearly; componentwise version parsing.

Container gates on the A0 tree (branch `handover/h0-bundle-import`,
2026-09-24 07:36–07:56 UTC): **test** PASS 2m08s (real rebuild — the
source change invalidated the cached layers, confirming the baseline
passes above derived from pre-edit content), **ts-test** PASS 3s,
**e2e** PASS 14m26s (incl. the locked-bottle offline reconstruction
fixture — locked pins bypass selection, transport unchanged),
**model** PASS 3s (spec unchanged), **verify** PASS 2m39s. Host:
`cargo test -p gripsack-fetch` 61/61 (13 bottle-selection cases).

Post-hardening integration smoke on 2026-09-24 (12:14–12:34 UTC):
`docker compose run --build --rm` **test, ts-test, e2e, model, verify**
all PASS after the bounded macOS probe and delivery-checker module split.
The Rust test gate ran fmt/clippy/tests including 61 gripsack-fetch tests,
e2e executed 245 cases, and Verus executed 56 obligations plus four
mutants. TypeScript and TLC `RUN` layers were CACHED. These are
unbound worktree observations in `/tmp/gripsack-integrated-gates/`,
not a new verified delivery claim or a substitute for required native
Mac/TLAPS evidence.

After adding the A1-07 Cargo dependency guard, the same five compose
gates passed again on 2026-09-24 (12:39–12:56 UTC): Rust fmt/clippy/tests
and the real e2e flow ran; TypeScript, TLC and Verus image layers were
CACHED. `check_architecture.py --self-check`, delivery inventory
validation and all 15 negative calibration cases also passed. This is
regression smoke, not a release closure claim.

## Record

| Item | Status |
|---|---|
| H0-01 reconciliation | implemented_unverified — reopened 2026-09-24 (`invalid_evidence`): the work was done and gates were observed, but the claim cited this Markdown document, not a bound runner report; baseline `test`/`ts-test` logs show CACHED layers and no log carries a commit marker. Prior claim preserved in the ledger's `historical_claims` |
| H0-02 inventory | in_progress — reopened 2026-09-24 (`invalid_evidence`): 149 rows lacked lanes/cases and 177 lacked evidence kinds. Original-spec platform/capability lanes, conjunctive case/proof inventories and runner/formal/review evidence kinds are now registered for **all 178/178** IDs, including the four former A1 placeholder rows; zero fields remain unregistered. This data does not verify the source-bound semantic inventory review, runner or global/native/proof acceptance; no new row verified and H0 remains open |
| A0-01 implementation | implemented_unverified — reopened 2026-09-24 (`invalid_evidence`): unit + container gates observed green (archived `verification/reports/2026-09-24-a0-*.log`: cargo test executed, gripsack-fetch 61 passed / 0 failed incl. 13 bottle cases; e2e 245 passed; verify 56 verified 0 errors + 4 mutants; ts-test/model CACHED) but the logs lack commit/dirty binding and the evidence was Markdown |
| A1-07 architecture/gate wiring | partial: edition-5 178-ID fingerprint, required-lane/global-gate closure, exact checkout or documented identical-source reuse, 15 calibrated negatives and `check_architecture.py --self-check` (forbidden direct and aliased target-specific crate edges), both wired into protected CI `test`; v3 schema/parser parity is exercised by the existing Rust acceptance corpus. H0-02 registration is complete, but a source-bound review and stronger per-kind/proof-obligation enforcement in the checker and verified H0/CI/platform lanes remain open; v4 schema and caller cutover remain open |
| Milestone closures | **retracted 2026-09-24** — H0 and A0 are NOT closed. The earlier `--close-milestone {H0,A0}` pass rested on Markdown-only evidence and handwritten counts (incl. a G-03 `obligations.checked: 50` that contradicts the runner logs' `56 verified, 0 errors`); the ledger's global-gate attestation records were moved to `historical_claims`. Closure now requires source-bound runner evidence per lane plus passing global gates G-01–G-08 under the hardened checker |
| Next | enforce registered per-ID evidence kinds and expected proof obligations in the delivery checker with adversarial calibration; capture source-bound H0 inventory review/runner and H0/A0/global reports before attempting closure; continue A1 grammar, B0-01 portable evidence and plan/0048 NEXT. Native Mac/VM, registry and prover cases require independent real evidence |
