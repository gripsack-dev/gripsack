# 0002 — Sourcing: resolvers, transports, and fetchers

- Status: draft
- Date: 2026-08-22
- Amends: 0001 (§3.1 sources, §6 components)

> Rename note (2026-08-22): "sourcerer" → **fetcher**; plugin executables
> are `gripfetch-<name>`; the IR module field is `fetch`, not `source`
> ("source" is reserved). Resolver packages: `gripsack-fetcher-*`.

## 1. The decomposition

"How to source a package" is two problems, and conflating them is what makes
package-manager plugin systems sprawl:

- **resolution** — deciding *what* to fetch: querying a registry API,
  picking a version, mapping an abstract name to a concrete artifact.
- **transport** — getting the bytes: HTTPS, git, mTLS, a signed URL, an
  internal CA.

They happen at different times in the gripsack model (0001 §4):
resolution at **eval** (Python, trusted, credentialed), transport at
**execute** (Rust core). Keeping them separate is what lets each stay small.

## 2. The escalation ladder

Use the lowest rung that works:

1. **Built-in fetcher with arguments.** Core fetchers (`tarball`, `git`,
   `github_release`, `cargo`, `file`) take overrides first:
   `base_url` (GitHub Enterprise is `github_release` with a different
   `base_url`, not a new fetcher), static headers, CA bundle. Covers most
   "internal mirror" cases.
2. **Python resolver at eval.** Arbitrary registry logic — internal
   Artifactory/Nexus APIs, version policy, SSO-token URL signing — is plain
   Python in the env repo's `lib/` or a pip package. It returns a **pinned
   fetch descriptor** (fetcher + URL/rev + hash). The core never learns
   anything new; the IR already only carries pinned sources.
3. **Fetcher plugin at execute.** Only when the *transport itself* is
   bespoke: mTLS, non-HTTP protocols, credentialed redirect dances. A
   separate executable the core drives over stdio.

Rungs 1–2 need no new machinery and cover the large majority of
internal-registry reality. Rung 3 exists so rungs 1–2 never have to grow
tentacles.

## 3. Resolvers (eval time)

- A resolver is ordinary Python: `(request) -> FetchDescriptor`. It runs
  with the user's credentials and environment — internal SSO, netrc,
  tokens. **The core never sees credentials.**
- Distribution: env repo `lib/`, or pip packages (PyPI or an internal
  index) declared under `[eval] deps` in `env.toml`. This is "the system
  acquired a new skill": the env repo is self-describing, including its
  sourcing logic — machine B clones the repo and has the skill.
- **Pinning rule**: a resolver MUST return an immutable reference
  (version, digest, rev). It SHOULD return the content hash; if it can't
  know the hash upfront, the core records it in the lockfile on first
  fetch (trust-on-first-use) and every later run verifies against it.
  Hash drift = hard error until `grip update` re-resolves. Invariant 6
  (0001 §9) survives intact: the lockfile remains the sole source of
  resolution.

## 4. Fetchers (transport plugins)

- An executable named `gripfetch-<name>`, discovered on `PATH` (git
  remote-helper style) or declared explicitly in `env.toml`
  (`[fetchers.<name>]`). Any language.
- Protocol: NDJSON over stdio, same family as rootle's provider protocol.
  Three operations to start:
  - `fetch {args, dest_dir, locked}` → writes bytes under `dest_dir`,
    responds `{sha256, provenance}`
  - `capabilities` → declared feature set (for `plan`/doctor output)
    **including its rate budget** — the fetcher knows its backend's
    limits better than the core does (0007 §throttling).
- IR node: `{"kind": "plugin", "name": "<name>", "args": {...}}` — opaque
  to the core; the store-path hash covers name + args.
- **The core verifies.** Returned bytes are hashed and checked against the
  lockfile before anything enters the store. Plugin output is never
  trusted: a fetcher can be wrong or malicious and the worst outcome is
  a failed apply, never a poisoned store.
- Failure modes, decided now: plugin missing → `plan`-time error with
  provenance pointing at the module line; hash mismatch → hard error
  (upstream mutation or tampering), `grip update` to accept deliberately.

## 5. Fetcher tiers

**In-tree (first-class, maintained in the core):** `file`, `tarball`,
`git`, `github_release`, `brew` (bottles), `pixi` (conda packages).
`mise` is deliberately absent — its backends are mostly GitHub releases,
which `github_release` already covers. Version pinning: git revs and
lockfile content hashes are trivial; brew/pixi pin via their own
lockfiles and bottle/package hashes, captured into our lockfile at
update time (0008 §5).

**Out-of-tree (`gripfetch-*` plugins):** distro packages (apt/dnf —
their pinning story is repo snapshots and maintainer scripts, not ours
to own), internal registries, anything bespoke. The plugin protocol is
the permanent home for the long tail; in-tree is earned by being
boring and universal.

## 6. Where things live

| what | where |
|---|---|
| built-in fetchers + plugin host + hash verification | `gripsack-dev/gripsack` (core) |
| official fetchers (artifactory, nexus, s3, …) | `gripsack-dev/gripfetch-*` repos |
| official resolver libraries | pip packages `gripsack-fetcher-*` |
| company-private resolvers/fetchers | the company's env repo or internal index |

Fetchers are pinned like everything else (git URL + rev, or a versioned
package) — but because transports are hash-verified, a fetcher's version
does **not** participate in store-path identity: content is content.
Resolution behavior is captured by the lockfile as usual.

## 7. Non-goals and adjacencies

- No sandboxing of resolvers or fetchers (consistent with 0001 §2.3):
  they are trusted code, same as modules.
- Builders get the same treatment later (in-process builders now,
  `custom_shell` escape hatch, plugin builders if reality demands).
- Resolver/fetcher auth is always ambient (env, files, SSO brokers);
  gripsack defines no credential store of its own. (Secrets-in-dotfiles is
  a separate future doc.)

## 8. Resolution lives in the core (amendment)

Built-in fetcher resolution ("latest release", tag listings) happens **in
the core at lock/update time**, not in frontend code at eval time. This
keeps all built-in API traffic inside the engine's throttle domains and
retry policy (0007 §throttling) — eval-time Python/TS resolvers remain
possible for custom registries but are outside the throttle by nature.
Rate behavior: honor `Retry-After` on 429 within the step's retry budget
before failing; throttle domains are token buckets per host
(`[throttle]` in `env.toml`), conservative built-in budget for
`api.github.com`.
