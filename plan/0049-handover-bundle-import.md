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
A source-bound structural runner at `953724a` now checks 178/178
unchanged source IDs/fields and eight checksum-covered bundle members;
it does **not** establish that every paraphrased case is semantically
complete. A source-bound semantic review of paraphrased cases,
explicit proof catalogs for every formal row and H0/A0/global/platform
results remain open; H0-02 is not verified. A1 (A1-07) implements
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

The earlier checker accepted `runner`, `review` and
`mutant-calibration` evidence but not `formal`. Its hardcoded
`PROOF_ROWS` named **22** of the **39** rows requiring formal evidence;
the remaining 17 could inherit a runner's self-reported count. It also
required a runner for five review-only rows. The checker now requires
every declared evidence kind per claimed lane and admits a review-only
row without inventing a runner. A non-global formal row declares
`proof_obligation_inventory` and a positive
`proof_expected_minimum`; global G-03 declares a distinct named
catalog/minimum for **each owning milestone**. H0-02's inventory
closure requires even future milestone catalogs, but closure of an
early A0 proof can never borrow or require a later A1/A2 theorem.
The canonical digest includes row ID, names, minimum and (for G-03)
milestone; it must match formal evidence and occur in its runner
bytes. Numeric passed/proof counts require labeled markers, not a
matching digit buried in a SHA-256 digest. Synthetic H0/A0 with
distinct proof names per milestone and review-only positives pass;
**27** negatives reject invalid/mismatched kinds, names, floors,
catalogs, counts and source. This is checker hardening, **not** a
proof: all 39 real formal rows still lack reviewed names/floors;
H0-02 and milestone closure remain open.

A targeted comparison against immutable Epic A §§3.2–3.3 and the
A1-01/A1-08/A1-11 rows found three inventory gaps. A1-01 named
only historical v4 catalog conformance even though the active
typed writer/reader is v5; its case now requires v5 and retains
strict read-only v4. A1-08/A1-11 described **unimplemented** work
instead of naming required positive cases; they now name identity
and context admission, bounded tree/child ownership, rendered-result
retention, old source/check stages, managed-block foreign bytes
and production proof targets. No result or row status changed.
This does **not** constitute independent semantic review of all
178 paraphrased rows.

The current committed-source checker receipt
`verification/reports/2026-09-26-h0-catalog-0be1eaa.log`
(SHA-256 `e04f648f6ba04cf2ad6afce5a0704bb068000b2fdd3ed262f02f83149cacc5b1`)
binds source `0be1eaa2d76b91152228fb124025e97389c028e6`,
fingerprint `32d73e886142cb8e758222128128faf9368f9ccf417a146437fa716cb0fcc31b`
and clean tracked roots before/after direct execution: **27/27**
negative calibrations rejected, synthetic H0/A0 and review-only
positives accepted; actual H0 closure rejects missing G-03/H0 proof
catalog and pending H0-02. Earlier `56b1581`, `52943b2`,
`70c5502`, `8c27693` and `ace9496` reports are historical.
Required CI
`36230541824` failed on source `52943b2` with `ETXTBSY` in a self-update
test fixture; `70c5502` staged and renamed that fixture before spawn.
Its exact-source manual CI
[`36231520581`](https://github.com/gripsack-dev/gripsack/actions/runs/36231520581)
then passed Linux test, native arm64 Mac e2e **319/319**, audit,
fuzz and docs, with native job bytes archived as
`verification/reports/2026-09-26-h0-macos-ci-70c5502.log`. That older
run does not qualify this newer source, enforce PR protection, or
supply Mac-VM, TLAPS or the missing formal catalogs.

## A3-01 — external TypeScript example CI reveals an unpinned Pixi result

Draft PR #164's real
[`typescript-env` job](https://github.com/gripsack-dev/gripsack/actions/runs/36223635427/job/108353417188)
at merge SHA `c833a5188586869194b0a45b3ff0bd7e16fe8fe3`
built the musl CLI and TypeScript package, typechecked the separate
`gripsack-dev/example-env-typescript` checkout and passed `grip check`
for seven modules. `grip apply` then failed safely with E301 for
`pixi("ripgrep")`: the external example's `locks/laptop.lock` expects
tree SHA-256 `e1d59570954a22ca864004229d6f64764e9f38fd50a673ad25686a19654cb5af`,
but the real fetched tree hashed
`6d8dfc5d348b61743ca908598978dc544737e450c4424bd3c56a8aadeede81dc`.
The shipped `gripsack-fetch/src/fetch/pixi.rs` always does a private
`pixi global install --force-reinstall`; the example names no resolved
version or complete transitive archive lock. This **does not** authorize
replacing the expected digest with observed bytes, ignoring the
failing job or advertising A3 parity. A3-01/A3-02's complete frozen
Rattler lock and native prefix acceptance remain pending; fix the
producer contract, independently validate bytes, then update the
separate example repo through its own review and rerun real CI.

The independent RUSTSEC-2026-0285 patch is now merged to protected
`main` through [PR #165](https://github.com/gripsack-dev/gripsack/pull/165)
as `db32f2050d19d1d55504f27e7a753432b9ca61e9` (2026-09-26).
The required `test`, native `e2e-macos`, `audit`, `fuzz` and `docs`
checks passed. Its separate `typescript-env` check still failed on
the same Pixi example lock mismatch, not on the rustls 0.23.45
security update. Merging this isolated patch neither repairs the
example nor qualifies A3-01/A3-02 or a public release.

## E0-02 — OS scheduling qualification (systemd lane)

The 2026-09-24 persistent-unit fixture was observed, but its
Markdown-only record could not verify the lane. A new exact-source
real user-manager fixture at `0be1eaa` is archived as
`verification/reports/2026-09-26-e0-systemd-0be1eaa.log`
(SHA-256 `932a36f8a76dee63bda0adbe85b9f8825ec41c3754d9c712aa3a61fa6a84b323`):
systemd 255, UID 1000, manager running, Linger=no; `daily`
normalizes to `*-*-* 00:00:00` local BST. A uniquely named
transient `.timer` bound a finite user `.service` with
`Persistent=no`, `AccuracySec=1s`, `Restart=no`; it fired
1.007 seconds after the requested local second, then both units
were `not-found`, absent from timers, and no persistent files
or user schedules were modified. The `systemd-linux` lane is
verified, **not** E0-02 as a row: launchd/macOS is **blocked**
without native Mac hardware. E3 Gripsack job registration and
sleep/reboot/DST behavior remain separately required.


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
| H0-02 inventory | in_progress — reopened 2026-09-24 (`invalid_evidence`): 149 rows lacked lanes/cases and 177 lacked evidence kinds. Original-spec lanes, conjunctive cases/proofs and runner/formal/review kinds are registered for **178/178** IDs. The exact-source structural report `verification/reports/2026-09-26-h0-inventory-953724a.log` (SHA `320e84504d7ab77f4f4b8d8ccdfc62e435cc878b5207317d9d4ed204d0905b9d`) proves original identity/field comparisons and 8/8 checksums, not semantic completeness. The checker now enforces per-kind lane coverage and predeclared named proof IDs/minima on closure; **all 39** real formal rows still lack semantically reviewed explicit proof catalogs. Original-case review, H0/A0/global reports and native/proof acceptance remain open; no row promoted to verified |
| A0-01 implementation | implemented_unverified — reopened 2026-09-24 (`invalid_evidence`): unit + container gates observed green (archived `verification/reports/2026-09-24-a0-*.log`: cargo test executed, gripsack-fetch 61 passed / 0 failed incl. 13 bottle cases; e2e 245 passed; verify 56 verified 0 errors + 4 mutants; ts-test/model CACHED) but the logs lack commit/dirty binding and the evidence was Markdown |
| A1-07 architecture/gate wiring | partial: edition-5 178-ID fingerprint, required-lane/global-gate closure, exact checkout or documented identical-source reuse, **27** calibrated negatives and a review-only positive (current source-bound `0be1eaa` report above), plus `check_architecture.py --self-check` wired into required CI `test`. Formal evidence is distinct from runner; every declared kind is conjunctive per claimed lane. Named proof IDs, labeled checked count and canonical row ID/names/minimum/**milestone** digest must occur in repo-local runner bytes and match the declared catalog; an A0 proof cannot impersonate H0 and a SHA-256 digit cannot stand in for a passing count. The live inventory still lacks all 39 reviewed proof catalogs and independent semantic H0 case review; H0/CI/native/VM lanes, schema and callers remain open |
| Milestone closures | **retracted 2026-09-24** — H0 and A0 are NOT closed. The earlier `--close-milestone {H0,A0}` pass rested on Markdown-only evidence and handwritten counts (incl. a G-03 `obligations.checked: 50` contradicting the runner logs' `56 verified, 0 errors`); those claims were moved to `historical_claims`. Closure requires source-bound reports for **each declared kind and lane**, named proof catalogs/floors where formal evidence is mandatory, and passing global gates G-01–G-08. Existing checker calibration is synthetic, not a completion report |
| Next | name and review proof obligations plus count minima for all 39 real formal rows; obtain a source-bound semantic H0 inventory review and H0/A0/global reports before attempting closure; continue A1 grammar, B0-01's required CI/container-gate evidence and plan/0048 NEXT. B0-01 Linux real-daemon qualification is source-bound at `0be1eaa`, not B0 closure; native Mac/VM, registry and prover cases require independent real evidence |
