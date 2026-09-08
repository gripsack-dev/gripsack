# 0044 — Complete surveys, explicit version spellings, and bounded HTTP failures

Status: **implemented for core/SDK 0.39.0**. Review: continued use of 0.36.0 →
0.38.0, received 2026-09-08. Decisions and executable bounded models preceded
runtime changes. The owner requested implementation, green CI, website
deployment and matching core/SDK publication after verification.

## Evidence and disposition

The reporter verified the following fixes; accept that evidence rather than
rerun the reported conditions:

| Closed item | Evidence accepted |
|---|---|
| Installed-frontend doctor | Stale 0.17.5 install produces MISS; removing it restores check for 40 modules; public npm has 0.38.0 |
| Executable templates | Fresh output is 0755 without the chmod workaround |
| Read-only update | Matching pin exits 0; a would-be change exits 1; lock stays clean |
| Tuicr tables | Pinned v0.22.0 is silent on both declared hosts |
| Complete update pins | 0.38 update computes tree256 without a subsequent apply; values agree with the laptop's completed pins |

The environment evidence is laptop/WSL Ubuntu 24.04, glibc 2.39. The Spaces
lockfile was resolved from that laptop; **no RHEL/Space deployment was verified**.
The private npm mirror remaining at 0.17.9 is not a gripsack release defect.

The attachment's introduction says one new finding, but its body contains four
numbered findings and a carried merge concern. All are dispositioned below.

| Review item | Decision |
|---|---|
| 1a: first failure hides the remaining survey | Adopt B: complete per-module results for `update --check` |
| 1b: exit 1 means both changes and failure | Adopt B: 0 current, 1 changes, 2 incomplete/error |
| 2: version spelling differs by surface | Adopt C: explicit `{version.bare}` without changing existing `{version}` semantics |
| 2: preflight after acquisition | Adopt C for decidable source-only paths, **before** publication; report recipe-dependent checks as deferred |
| 2: guess archive shape without downloading | Reject a generic heuristic: release tags and archive layouts legitimately differ |
| 3: infer credential authority from `base_url` | Reject implicit binding; adopt D's precise host-binding diagnostics and honest docs |
| 3: enterprise API-vs-browser download explanation | Adopt D, including warm/cold locked apply and HTML/login responses |
| 3: source `/etc/profile.d` automatically | Environment responsibility; document the boundary, never source shell profiles in grip |
| 3/4: reuse gh credentials | Defer an explicit opt-in, host-scoped provider; roadmap D1 |
| 4: transient asset failures and retry evidence | Adopt E: classified, bounded idempotent HTTP retries and terminal attempt evidence |
| 4: shared-IP anonymous API exhaustion | Adopt D diagnostics/docs; no claim that a local throttle can repair an upstream shared quota |
| Carried merge concern | Partly already handled; adopt A's remaining **P0** boundary and evidence fixes |
| Recipe-produced payload layout before building | Defer stronger read-only prediction, not existing apply-time verification; roadmap D2 |

## A — P0: one merge inspection boundary, no unowned-tail deletion

### The 0.38 boundary defect

Complete duplicates are no longer invisible: `ops/plan.rs::plan_merge` requires
one block for satisfaction, and the existing duplicate flow test covers an
edited second block. Replacing content inside complete managed blocks is the
existing self-healing contract, not a new authorization to delete foreign text.

However, `template.rs::find_blocks` returns complete pairs while `upsert_block`
rescans raw opening markers during cleanup. An unmatched second opener is absent
from inspection but starts an unbounded skip during rewriting. On a payload
update, the following unmanaged tail is deleted. This is a root boundary defect,
not a diagnostic-only issue. `marker_mode` and `merge_notes_pub` also inspect only
the first block; conflicting metadata can be hidden by ordering.

A sandboxed real-0.38 CLI probe in this planning round observed:

| Input | Current result |
|---|---|
| Edited second complete block | Reconciled to one; foreign tail retained; duplicate removal reported, edit classification omitted |
| Correct-mode block before conflicting-mode duplicate | Rewritten to one block |
| Same blocks with conflicting-mode duplicate first | Preserved byte-for-byte as drift |
| Valid block + unmatched second opener + foreign tail; payload changed | Apply succeeds, **foreign tail disappears**, no duplicate report |

This probe creates only temporary fixture homes; it does not touch the reporter's
files. Until A lands, repair unmatched managed markers before updating a merge
payload. No claim of a production fix is made by adding the model.

### Required change

- Introduce one linear, lossless scan yielding complete owned ranges and explicit
  malformed/nested/interleaved-marker errors. Inspection, satisfaction, rewrite,
  prune, and reporting consume that same result. No second raw-opener scan.
- Unmatched or ambiguous boundaries fail before mutation/journaling; no guessed
  recovery, no success receipt, no truncation of the remainder of the file.
- Preserve foreign bytes and their relative order. Do not use whole-file
  line-joining or broad trimming to erase text outside established owned ranges;
  retaining an extra separator is safer than claiming ownership of user whitespace.
- Examine **every** complete block's mode/hash evidence. Any known mode conflict
  prevents rewrite regardless of block ordering. Legacy mode-less markers retain
  0043's explicit upgrade rules; unknown metadata never manufactures prune authority.
- Preserve the intended reconciliation of complete managed duplicates. Report
  duplicate count and edits in any block, not just the first. Prune remains
  conservative: a single intact, authorized block; duplicates/ambiguity are kept.
- Leave cross-module aggregation deferred; E111/E119 continue to reject sharing.

Ownership: rendering `{{ vars }}` remains in `template.rs`; cohesive
`managed_blocks/{mod.rs,parse.rs}` owns marker parsing, inspection and splicing.
Use descriptive `ManagedBlock`, `ManagedBlockSet`, `MergeInspection`, and
`MergeParseError` roles; borrow byte ranges/content rather than duplicate buffers.
Migrate planner, executor, removal, fuzz, and tests together; remove superseded
first-block helpers/re-exports rather than retaining a compatibility API.

Proof: `MergeBoundary.tla`; future Rust explorer drives the shipped scanner and
planner over 0–3 blocks, later edits/mode conflicts, legacy markers, unterminated
and interleaved markers, LF/CRLF and arbitrary foreign prefix/interstitial/tail
bytes. Keep the real unclosed-tail reproduction as a failing-before/passing-after
regression. Test apply and prune, repeated apply, rollback/compensation, and the
fact that malformed input never commits an apparently successful generation.

## B — Complete read-only surveys, unambiguous exits

`gripsack-exec/src/update.rs::update` currently accumulates reports but propagates
source/overlay/cache errors with `?`. The CLI only renders after `Ok(reports)`;
one later error therefore hides earlier successes too. `UpdateStatus` has no
failure variant, and command setup, resolution failure and drift all reach exit 1.

- Separate preparation of one source from the driver that decides publication.
  Reuse `PreparedModule`, source acquisition, overlay, and complete-pin logic.
  Keep normal `update`'s fail-fast/one-lock-write policy; this is not partial apply
  or a request to commit only successful pins.
- In `--check`, every selected module gets an outcome: unchanged, would-change,
  failed, or not-applicable (no fetch). Deduplicate the existing scoped selection;
  preserve stable ordering. An explicitly requested unknown module is incomplete,
  not a successful skip. Do not skip independent modules because another failed.
- Retain failure phase, structured cause and provenance. Extend existing report
  types with a descriptive failure variant; do not flatten errors into strings
  before retry/auth/preflight classification. The renderer alone chooses prose.
- Print per-module failures alongside successes and finish with counts. A summary
  containing failures must say **incomplete**, even when other modules would change.
  Only a complete survey can claim all applicable pins are current.
- Global trust/eval/IR/lock/setup errors abort with a diagnostic before a survey.
  Interruption and fatal shared-resource failures must not claim completion; retain
  already available results when possible and name unattempted work explicitly.
- A known host cooldown can yield a named, zero-attempt unavailable result for
  affected modules rather than hammering that host. Other forges still proceed.
- Every stage is private and promptly dropped; a survey publishes no source cache,
  lock, generation or destination. Do not retain all payloads to render a summary.
  Normal runtime provisioning/run logs retain 0043's documented behavior.

| `update --check` result | Exit |
|---|---|
| Complete, no changes (including no applicable sources) | 0 |
| Complete, at least one change | 1 |
| Any failed/unavailable/unknown requested module, or global operational/setup failure | 2 |

Clap usage errors already use 2; preserve conventional signal termination rather
than disguising cancellation as drift. Update automation docs: callers must now
distinguish 1 from 2. Other commands' exit contracts are not silently changed.

Ownership: `update/{mod.rs,prepare.rs}` if extraction warrants the split,
`report.rs`, and `commands/update.rs`. Prefer named `UpdateMode`,
`UpdateModuleFailure`, and `UpdateCheckOutcome` domains to a growing boolean/tuple
interface. Compute summary from results once, not two divergent status lists.
No new generic scheduler, parallel network fan-out, JSON flag, or retrying recipes.

Proof: `UpdateSurvey.tla` checks completion, accounting, no publication, and error
over change precedence. Future Rust tests drive the real result reducer; offline
flows use successful/moving/401/500 modules in every position, including a prior
success followed by failure, selection, empty graphs, unknown names, and corrupt
locks. Assert public results and byte-identical lock/cache state, not forwarding.

## C — Explicit version spelling and exact source preflight

`resolve.rs::expand_asset_pattern` tries the raw tag then one-lowercase-v-stripped
form; `pick_asset` still stores the **raw tag** in the lock. `source.rs::payload_source`,
`verify.rs::run_verify`, and `report.rs::describe_verify` independently substitute
that raw value. Asset matching is compatibility search, not a universal path value.

Adopt **`{version.bare}`**, not a boolean threaded through each fetch shape:

| Locked version | `{version}` in paths | `{version.bare}` everywhere it is supported |
|---|---|---|
| `v0.12.1` | `v0.12.1` | `0.12.1` |
| `0.12.1` | `0.12.1` | `0.12.1` |
| `vv1` | `vv1` | `v1` |
| `V1` / `release-1` | unchanged | unchanged |

Strip exactly one leading lowercase `v`, not arbitrary letters or every `v`.
Missing version is unresolved, never empty/current/latest. A version consisting
only of `v` cannot satisfy a required bare expansion. Validate expanded paths and
containment; symbolic admission is not permission to trust arbitrary resolved text.

- Preserve existing `{version}` asset fallback and raw path behavior. Patterns
  containing only `{version.bare}` have one exact version spelling; do not restore
  a `v` as an implicit fallback. If both tokens occur, bare stays fixed while only
  the legacy raw-token asset candidate varies. Deduplicate identical candidates.
- One shared expansion policy serves asset patterns, payload source construction,
  verify paths and rendered diagnostics. No apply-only filesystem probing/fallback,
  no storing the winning asset spelling as a different lock version.
- Apply the token to all current eligible data/explicit-step install/config `from`
  and payload-verification slots. Keep scripts/argv, destination paths, git refs
  and opaque plugin arguments under their existing contracts; this feature does
  not invent a locked version for fetchers/slots that have none. Unsupported
  contexts must be explicit diagnostics rather than unresolved braces reaching I/O.
- Keep repo overlay capture's existing selected-input contract. Preflight examines
  the **actual captured-and-merged stage**, not a fresh read from the checkout;
  do not reinterpret overlay keys or broaden overlay ownership to implement this.
- Update the schema contract/docs, Rust admission (including explicit-step entry
  coverage), TypeScript API docs/emission tests and golden examples together.
  This additive token does not reinterpret old IR, so IR stays v3. Old cores may
  reject the new token; document the minimum supporting release when it ships.
  Review recipe/input hashing; raw lock tags and old manifest concrete paths stay
  unchanged. No lock migration or new identity namespace.

Example authoring (illustrative planned syntax, not usable on 0.38):

```ts
const PAYLOAD = "rootle-{version.bare}-{target}";
fetch: githubRelease({ repo: "rootledev/rootle", asset: `${PAYLOAD}.tar.gz` }),
install: { [`${PAYLOAD}/rootle`]: symlink(`${p.bin}/rootle`) },
```

Preflight **after resolution/acquisition/overlay, before cache and lock publication**:
source-only modules can verify every concrete install/config source and the
existence/type precondition of payload `fileExists`/`binaryRuns` paths. Never run
binaries or shell verification here. Known missing/invalid paths are module
failures, not a warning followed by committing an unusable pin. Name the original
pattern, locked tag, concrete relative path, observed top-level entries and a
`version.bare` remedy when that mismatch is actually established.

`check`/`plan` remain offline: use an already available, matching artifact when
there is one; otherwise report deferred layout evidence, never claim payload
existence. Recipe-produced paths, runtime verification and deployed destinations
are explicitly deferred to their existing apply-time boundary. Stronger read-only
recipe layout prediction is roadmap D2. A tag prefix alone cannot prove an archive
layout, so a generic no-download `strip_v` warning would recreate noisy-linter debt.

Ownership: common token grammar in `gripsack-ir` and cohesive expansion in
`gripsack-fetch/src/placeholders.rs`; keep GitHub's legacy candidate ordering in
its resolver. Existing `PayloadSource` owns the concrete source path; a small
`source/preflight.rs` owns staged-layout evidence. Remove independent replacement
closures from verify/report. Avoid a second path language or generic templating engine.

Proof: deterministic Rust expansion/consumer matrix across raw/bare/mixed/repeated
and unsupported tokens, prefixed/unprefixed tags, traversal/empty expansions,
platforms, bare binaries, nested archives, repo overlays and explicit steps.
Use an independent expected table plus the **shipped** helpers and CLI, not
self-comparison. Include warm/cold apply, no-op reapply and historical rollback.
String expansion needs these bridges more than a TLA model of opaque strings.

## D — Host-bound authentication, actionable 401/403/login failures

`http.rs::Policy::header` binds the public token to github.com/api.github.com and
an enterprise token only to the host named by GH_HOST/GITHUB_HOST.
`fetch/tarball.rs::reader` chooses `api_url` only if a credential binds to it;
otherwise it tries the browser URL. Locked GitHub specs lower to Tarball specs,
so useful non-secret GitHub request context must survive that lowering.

**Retain explicit token audience.** `base_url` and returned `api_url` describe
where a module wants traffic, not where an ambient secret may be sent. Automatic
binding to any declared enterprise-looking URL is rejected. GH_HOST is a stricter
gripsack binding requirement, not exact gh-CLI parity; gh can infer hosts from
other command/repository context. Current gripsack token precedence also differs
from gh (GITHUB_TOKEN before GH_TOKEN, likewise enterprise aliases); do not silently
change that precedence in a diagnostics patch.

- Carry typed, non-secret request purpose and authentication disposition through
  resolution and locked cold fetch. Distinguish no credential, enterprise token
  present but unbound, binding to a different host, and a bound token rejected by
  the server. Names such as `AuthenticationDisposition`, `GithubRequestContext`,
  `HttpFailure` and `RetryStopReason` should say what the values mean.
- On enterprise 401, and on an HTML/login asset response, name the actual selected
  request URL and the declared API host. With an unbound/mismatched enterprise
  token, suggest `GH_HOST=<that-host>` plus the applicable enterprise token names.
  With a correctly bound rejected token, discuss scope/SSO/expiry, not rebinding.
  Do not fail an otherwise successful anonymous public/GHE request just because
  a token is absent. Do not turn all 404s into "release does not exist" certainty.
- For api.github.com 403/429, inspect bounded headers (`x-ratelimit-remaining`,
  `x-ratelimit-reset`, `retry-after`) and bounded relevant error metadata. Label
  primary rate exhaustion only when evidence supports it; other 403s can be
  permissions or secondary limits. Without a bound public credential, suggest
  GH_TOKEN/GITHUB_TOKEN and explain the IP-scoped anonymous quota. Do not promise
  any PAT bypasses SSO, secondary limits, proxies or all shared-egress restrictions.
- Preserve parsed-host matching, public/enterprise separation, API asset selection,
  no-proxy/system-CA behavior, and same-host/no-downgrade redirect isolation. A
  retry replays the original request under the captured policy, not a redirected
  URL with freshly acquired credentials. Redact userinfo, sensitive query material
  and headers; credentials never enter Debug, diagnostics, IR, locks or model data.
- Document non-interactive SSH: grip inherits the supplied environment and never
  sources `/etc/profile.d` or arbitrary shell scripts. Fix the caller's provisioning
  environment; the report's `install.sh` is not this repo's core bootstrap installer.

No gh file/keychain reading, automatic login, new token storage, proxy/IP rotation
or credential discovery from a module URL. Opt-in host-scoped gh integration is D1.
The HTTP guardrail does not sandbox trusted module shell effects.

Proof: `CredentialRouting.tla` rejects base-URL authority and redirect forwarding.
The future Rust bridge uses the actual URL parser/selector with canonical-host,
userinfo/backslash/suffix traps, ports, scheme changes, public/enterprise/no-token
cases, and redaction canaries. Local HTTP fixtures exercise metadata, API assets,
browser HTML and locked cold apply without real credentials or corporate services.

## E — Bounded idempotent HTTP retries, with terminal evidence

There is **no application-level 5xx retry policy** in the current resolver or
asset reader. Both call ureq once. Pinned ureq 2.12.1's `unit::connect_inner`
contains stale pooled-connection recovery; its "retrying request" strings do not
mean HTTP 500 is retried. Error flattening currently discards useful status/context.

The actual built-in pacing is api.github.com **30/min**, ghcr.io **30/min**, and
formulae.brew.sh **60/min**. github.com release downloads/CDNs have no default
bucket. These are local pacing budgets, not a measurement of an egress IP's
remaining upstream quota; do not retune them to claim to solve the Space's 403.

- One classified retry policy for first-party HTTP GET metadata and payload
  acquisition. Admit 500/502/503/504 and explicitly classified transient connection,
  timeout or interrupted-body failures. Do not retry certificates, auth/permission,
  absent assets, malformed requests/JSON, HTML/login pages, hash mismatches, archive
  safety/size/decoder failures or local filesystem errors as "transient".
- At most **3 policy attempts total**, not three retries; short exponential sleeps
  (1s then 2s absent server guidance), cumulative retry wait at most **30s**, within
  one unchanged **600s operation deadline** including throttle/requests/body reads.
  Never multiply the existing request deadline per retry. A long healthy asset
  transfer is not arbitrarily cut to 30s: 30s is the retry-wait cap, not payload time.
- Honor Retry-After/reset evidence as a lower bound. If it cannot fit the remaining
  budget, stop with a specific cooldown/budget reason; never clamp it downward and
  request early. Confirmed secondary-limit advice requires at least the documented
  minute when no stronger header is supplied, so it normally cannot fit this short
  retry-wait budget. Do not blindly retry a generic 403.
- Keep known per-command host cooldown evidence so B can account for unavailable
  same-host modules without a request storm. A module failure on another forge does
  not globally block healthy hosts. Make throttle waits deadline-aware and debit
  each policy attempt; preserve existing domain override precedence.
- Replay the same resolved source and expected pin, never resolve `latest` again
  during payload retry. Discard partial spools; restart at zero, not append/resume.
  Count actual bytes across retries against one acquisition budget; do not reset
  resource accounting. Only complete verified bytes reach extraction/publication.
- Terminal errors and trace events name safe original/effective URL when known,
  failure class/status, policy attempts, elapsed/wait time and stop reason. Report
  "1 attempt, non-retryable" distinctly from exhaustion. ureq's internal pooled
  reconnects are not observable policy attempts: do not claim this count is an
  exact count of TCP connections/wire transmissions.

Ownership: `http/retry.rs` owns classification/deadline decisions;
`http.rs` remains the per-context transport/policy entrypoint. Share typed failure
context with D and keep the streamed-spool seam in `fetch/tarball.rs`/`spool.rs`.
Avoid stacking resolver, fetcher and scheduler retries. No IR retry fields, recipe
replays, native-manager/plugin process replay, new global retry settings, or hidden
authentication fallback in this round.

Proof: `HttpRetry.tla` checks bounded attempts, fixed deadline, terminal exclusions,
throttle admission and termination under explicit clock/OS-progress assumptions.
Future Rust tests drive the shipped classifier with a fake clock; loopback flows
cover 500→success, exhausted 5xx, 401/403, Retry-After beyond budget, truncated
streams, checksum failure, deadline exhaustion and no partial publication. Header
classification and body limits require real transport tests, not only the model.

## Deferred roadmap entries

- **D1 — Opt-in, host-scoped gh credential provider / multiple enterprise bindings.**
  Both the enterprise and public-token suggestions refer to the same capability.
  gh normally uses the OS credential store, with plaintext only as a fallback;
  parsing `hosts.yml` is not a complete implementation. Revisit with explicit
  opt-in, host allowlisting, precedence, keychain/noninteractive behavior, redaction
  and a bounded provider protocol. Never silently read all available credentials.
- **D2 — Read-only evidence for recipe-produced layout.** Revisit when a concrete
  producer can provide a trustworthy, versioned output inventory without running
  recipes. Existing apply-time verification remains; C must say deferred, not
  validate a hypothetical source archive as though it were a build output.

Rejected proposals stay here, not disguised as future promises: implicit
base-URL token grants, changing existing `{version}` meaning, per-fetch strip-v
booleans, generic archive-shape warnings, automatic profile sourcing, and a claim
that local throttling can recover a shared anonymous GitHub quota.

## Implementation sequence and acceptance

1. Land A's data-loss guard and all-block evidence first; it is release-blocking.
2. C's expansion/preflight and D/E's typed HTTP boundary can proceed independently
   with named ownership. B consumes their typed per-module outcomes. One integration
   owner controls public exports, schema/frontend synchronization and report shape.
3. Keep focused modules, roughly below 800 production lines as in 0041, but split by
   responsibility rather than line count. Prefer descriptive enums/structs to
   boolean tuples or abbreviated domain names; do not wrap ordinary labels merely
   to increase type count. Remove migrated code and update every LSP-found caller.
4. The design models below are **not implementation proof**. Required Rust/CLI
   bridges must fail against the corresponding current defect and pass the change.
   Keep positive and named-negative configs; parser errors never count as mutants.
5. At implementation landing: Docker test/ts-test/e2e/model gates, native macOS,
   executable published examples, bounded fuzz, updated docs/STATUS/changelog and
   deliberate core/SDK versioning and verified publication. No RHEL/Space claim
   without a real corresponding deployment.

### Bounded design evidence in this change

| Model | Positive contract | Calibrated negatives |
|---|---|---|
| `UpdateSurvey.tla` | Three selected modules in every outcome/completion order, plus empty selection; complete accounting and exit reduction; no publication | stop on first failure; conflate exits; publish during survey |
| `HttpRetry.tla` | GET/non-GET classes, three attempts, abstract operation clock and sleep budget | replay terminal failures; reset deadline; fourth attempt |
| `CredentialRouting.tla` | Explicit public/enterprise audience; presence/missing binding; same-host/no-downgrade redirects | bind from declared URL; forward across redirect |
| `MergeBoundary.tla` | 0–2 block descriptors, known/unknown/conflicting modes, edited blocks, malformed boundaries, apply/prune and foreign text conservation | ignore unclosed opener; first-mode-only inspection; first-block-only prune |

The canonical `scripts/check_models.sh` runs these alongside existing models.
The new contracts have five positive configs and eleven named negatives. Network
classification, URL/marker parsing and real filesystem effects remain outside
these abstractions and are assigned explicit implementation bridges above.

### Sources

- Current symbols are named in A–E; existing contracts: plans 0001/0002, 0013,
  0016, 0036, 0041, 0042 and 0043.
- [GitHub rate-limit semantics](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)
  distinguish IP-scoped anonymous primary quota, other 403s, reset headers and
  secondary-limit cooldowns.
- [GitHub API best practices](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api)
  require honoring server cooldowns instead of immediate retries.
- [gh environment behavior](https://cli.github.com/manual/gh_help_environment)
  and [gh credential storage](https://cli.github.com/manual/gh_auth_login)
  explain why exact gh parity and plaintext-hosts-file reuse cannot be assumed.
- ureq 2.12.1 `src/unit.rs::connect_inner` and `src/agent.rs::RedirectAuthHeaders`
  distinguish internal stale-connection replay from a status retry policy and
  define the current redirect boundary.

## Implementation evidence

The implementation keeps the decisions above and separates marker parsing,
splicing, exact expansion, staged preflight, update preparation, HTTP request
control, response accounting, retry policy and redacted failure context.

- `managed_blocks/{parse,mod,tests}.rs` replaces first-block helper APIs;
  `template.rs` now only renders whole-file templates. The lossless parser drives
  planner, executor, prune and fuzzing. Preview errors propagate to failing exits.
- `update/prepare.rs` never publishes. The driver chooses `UpdateMode::Check`
  versus `Publish`; `UpdateSummary` reduces real outcomes with failure precedence.
- `placeholders.rs` centralizes exact source spelling and legacy asset candidate
  ordering. `source/preflight.rs` distinguishes acquired sources from available
  completed artifacts; verifiers are not executed for layout evidence.
- `http/{request,retry,body,failure}.rs` owns one GET operation deadline, aggregate
  transferred bytes, classified retries, cooldowns and redacted error context.
  The existing client retains parsed host binding, proxy/CA settings and ureq's
  credential-isolating redirects. Module context annotates cold GitHub failures
  without adding credentials or fields to the lockfile.
- Targeted container evidence before the version bump: 26 merge/ownership/preview
  flows, 27 survey/migration/pin flows, and 31 HTTP/survey/pin/self-update flows
  passed (these groups overlap; they are not a combined test count). The actual
  CLI smoke exercised mixed success/failure, all three exit codes, 500 retries,
  preflight failure/remedy, unchanged lock/cache, and no verifier execution.
- TLC's complete model gate passed all 60 configurations: the original 44 plus
  five new positives and eleven exact-invariant negatives. The negative cases
  calibrated the models themselves before they were used as evidence. Rust model
  bridges call the shipped reducer, retry decisions, host selector and block parser;
  local HTTP flows additionally prove real redirects, cold API selection and
  interrupted transfers. None of this claims a RHEL/Space deployment.
