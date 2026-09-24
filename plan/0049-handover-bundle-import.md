# 0049 — Handover bundle import (edition 5) and H0 reconciliation

Status: H0 in progress · Owner: implementation agent · Date: 2026-09-24

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

Run in a detached `git worktree` at the baseline commit so concurrent
implementation cannot contaminate the record (gates bake verification
into image builds from the live build context).

<!-- BASELINE_RESULTS -->

## H0-02 — delivery and support inventory

`verification/delivery.json` (format `gripsack-delivery-ledger` v1)
registers every ID with owner milestone, closure scope, source
document, verbatim deliverable/evidence, and honest status. Near-term
rows (H0, A0, B0, E0, A1 — 21 IDs) carry live platform/capability
lane registrations and conjunctive case inventories; all other rows are
`imported_pending_live_registration`, which blocks any closure claim on
them until their milestone registers lanes. A1 (A1-07) implements
`scripts/check_delivery.py`: inventory validation, scoped closure
checking, and the required negative calibration; H0/A0 evidence must
survive that checker or be repaired, not grandfathered.

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
Replaced by a pure policy module (`crates/gripsack-fetch/src/bottle.rs`)
over injected facts:

- `HostPlatform { os, arch, macos_version }` — detected once per
  `FetchContext`; the policy itself never reads the environment.
  macOS version arrives via `sw_vers -productVersion`; a missing fact
  refuses selection rather than guessing.
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

<!-- A0_GATES -->

## Record

| Item | Status |
|---|---|
| H0-01 reconciliation | done (this document) |
| H0-02 inventory | done (`verification/delivery.json`) |
| Baseline gates | see above |
| A0-01 implementation | done, container gates pending |
| Next | B0/E0 qualification records, then A1 (checker first) |
