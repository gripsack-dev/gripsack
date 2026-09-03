# 0020 — Review response: what we adopted, what we rejected

Source: an external architecture review of 0.18.1 ("high-quality
implementation, insufficiently seasoned safety core"). Every claim was
verified against the code before acting; the source-level findings
were all real and are fixed in 0.19.1. The product/architecture
proposals were triaged individually — several are rejected with
reasons, because the reviewer's own framing ("preserve the ownership
model, adoption flow, versioned IR, Rust core, diagnostics and
generation history") is the part that works.

## Adopted (shipped in 0.19.1)

| # | Finding / proposal | Resolution |
|---|---|---|
| 5.1 | Post-flip journal window: crash between the flip and journal cleanup made recovery restore priors a committed generation owns | **Fixed.** `journal::begin_run(home, target)` writes a run marker naming the target generation before any mutation; `reconcile` compares it against `current` — committed runs clean up, uncommitted runs restore. Unit tests pin both crash positions |
| 5.2 | Corrupt recovery metadata failed open (silently deleted) | **Fixed.** Malformed entries move to `journal/quarantine/` and reconcile ERRORS — recovery metadata is never archaeology. Fail closed, with the path and remediation in the message |
| 5.3 | Any `symlink_metadata` error read as `Absent` | **Fixed.** Only `NotFound` means absent; permission/I-O errors abort the transaction. (Recovery could have REMOVED a destination it could not read) |
| 5.4 | `to_string_lossy` on non-UTF-8 paths/targets | **Fixed** for the reachable case: destination paths are UTF-8 by IR construction (JSON strings); non-UTF-8 symlink TARGETS now refuse the mutation loudly instead of recording a target that would restore as a different link |
| 5.5 | EXDEV publish fallback copied directly into the final store name | **Fixed.** Copy to a temp sibling under the store parent (same filesystem), then rename — a crash mid-copy can no longer leave a partial "immutable" path |
| 5.6 | Plugin trust model overclaim; "core never sees credentials" imprecision | **Fixed** in docs: hash verification protects store contents, not the host — plugins are trusted code with user privileges; the credential boundary is eval-vs-everything-else (eval sees none; fetching necessarily can) |
| 5.7 | install.sh sorted by minor+patch, not major | **Fixed.** `sort -t. -k1,1nr -k2,2nr -k3,3nr` — 1.0.0 now beats 0.99.0 |
| §10 | Risk classification in plan output | **Adopted** (this release): every planned mutation carries its reversibility class — `reversible (prior recorded)`, `best-effort (adapter)`, or `no automatic inverse (runs custom code)` |
| §5.8 | Release integrity | **Partially adopted**: artifact attestations are the next supply-chain step; signed channel manifest/SBOM tracked below |
| §5.9 | macOS behavioral tests | **Adopted as roadmap**: the release matrix builds on macOS runners today; a macOS e2e job is queued behind it |

## Rejected (with reasons)

### A data-only config format (TOML/JSONC) as the default, TypeScript optional

Rejected. This reintroduces the two-frontend problem plan/0013 was
written to end (the Python frontend's parity corpus is the cautionary
tale), and it forks the product's one genuine language asset:
modules-as-values with factory composition — the style our most
serious real-world user independently converged on. A second format
means every sema pass, linter, span, and diagnostic exists twice or
degrades to the lowest common denominator. The 40MB Deno download is
once per machine, cached, and `grip doctor` reports it; the cost is
real but bounded, and the payoff is spans pointing at real source —
the thing a data format can never have. If the download ever proves
to be the adoption blocker, the honest answer is a pre-warmed store
or a bundled runtime option, not a second language surface.

### CEL (or any expression language) for conditions

Rejected. "No new language to learn" is a stated product principle;
CEL is a language. Hosts already express conditions as plain
TypeScript (`ctx.facts.os === "linux" && steam`), which typechecks,
has spans, and needs no new evaluator. The constrained part of the
evaluation story is the SANDBOX, not the expressiveness.

### Packages as coordinator-of-package-managers only

Rejected as a product definition, accepted as an integration pattern.
The critique itself preserves "the safest way to bring an existing
environment under management" — but for many users the environment
IS tool binaries, and `gripfetch-*` plugins already ARE the
coordinator pattern for ecosystem managers (apt exists as a plugin,
not in-core). First-party fetchers cover the standalone-binary long
tail (github releases, tarballs) that brew/mise handle poorly for
private/enterprise hosts — which is where our real-world usage
lives. What we take from the proposal: no NEW first-party fetcher
kinds until users ask (the freeze the reviewer asked for, kept).

### `undo` as a second name for rollback

Rejected. Two names for one destructive-ish operation is confusion,
not kindness. `rollback` is precise and documented; the plan output's
reversibility labels (adopted above) teach the actual semantics
better than a synonym would.

### cap-std/openat-style directory-capability filesystem access

Deferred, not rejected. It is the right long-term hardening for the
path-handling core, but it is a wholesale rewrite of the fs layer
while the crash-recovery protocol is days old. Queued behind: soak,
fault-injection breadth, macOS behavioral tests. Recorded here so
the decision survives.

### TLA+/PlusCal model of the transaction state machine

Deferred. The state machine is now small enough to state plainly
(see plan/0019's amendment: `begin_run → record → mutate → mark_after
→ flip → commit_run`, with reconcile deciding committed-vs-not from
`current` vs the run marker) and is exhaustively unit-tested at both
crash positions. A formal model is worth it when the protocol next
grows (it grew once already: the run marker).

## The invariant (restated, post-fix)

> After any crash and recovery, managed state is either the previous
> generation or the committed target generation — never an unexplained
> mixture — and post-crash user edits are never overwritten.

The run marker closes the last known hole in that sentence; the
quarantine makes the failure mode loud when the journal itself is
damaged.

## Queued (priority order)

1. GitHub artifact attestations in release-core.yml (supply chain)
2. macOS e2e job (the release matrix's runners, reused)
3. cap-std/openat fs layer (when the recovery core soaks)
4. Signed update-channel manifest + installer verification
5. SBOM (SPDX) per release
