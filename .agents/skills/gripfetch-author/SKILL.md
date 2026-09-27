---
name: gripfetch-author
description: Author a gripfetch-* transport plugin for gripsack — the protocol contract, the pinning story for your ecosystem, and the conformance suite that proves your plugin. Use when implementing a fetcher for a bespoke transport (internal registry, apt/dnf, mTLS, credentialed redirects).
---

# Authoring a gripfetch-* fetcher

You are writing a **transport plugin**: an executable named `gripfetch-<name>`
that fetches bytes the core's built-in fetchers can't reach — an internal
registry, a distro package mirror, anything with an mTLS or credentialed
dance. The contract is small and every rule below is load-bearing. The
conformance suite proves you got it right.

Study the request flow first (plan/0002 §4, 0009 §2): the core spawns you,
sends ONE JSON line on stdin, reads NDJSON messages on stdout, and
hash-verifies every byte you staged before it enters the store.

## 1. The contract (what the core guarantees and expects)

**stdin** — one JSON line:

```json
{"op": "fetch", "args": {...}, "dest_dir": "/abs/staging", "locked": {"url": "...", "version": "...", "sha256": "..."}}
```

- `args` is opaque to the core — your module's `pluginFetch("<name>", args)`
  verbatim. Version it yourself; the store-path hash covers name + args.
- `dest_dir` is your staging area. Write the payload tree under it. Nothing
  else on disk is yours to touch.
- **`locked` is present iff the lockfile has a pin for this module** (first
  apply after `grip update`, or any later one). Its absence means
  trust-on-first-use: resolve and pin. Its presence means *reproduce
  exactly* — for an internal registry those are genuinely different code
  paths (fetch "latest matching" vs fetch this exact artifact).

**stdout** — NDJSON, one message per line, then exactly one `response`:

```json
{"type": "diagnostic", "diagnostic": {"code": "W01", "severity": "warning", "message": "mirror b is stale", "labels": []}}
{"type": "progress", "current": 1048576, "total": null}
{"type": "diagnostic", "diagnostic": {"code": "A01", "severity": "error", "message": "artifact not found", "labels": [], "help": "check the repo path"}}
{"type": "response", "id": 1, "result": {"provenance": {"registry": "artifactory.internal", "artifact": "tools/grip/1.4/grip-1.4.tgz"}}}
```

- **Diagnostics are data, never stderr prose.** Codes are codespaced for
  you: emit bare codes (`A01`, `W01`) and they render as
  `gripfetch-<name>/A01`. Severity rules: `warning` flows and fetches
  continue; `error` fails the fetch. The core renders them with the same
  snippet/caret care as its own — give them `labels` with spans when you
  have a file to point at.
- **`provenance` is the valuable half of the response** — which registry,
  which mirror, which credential identity served the bytes. It lands in
  the run log (0009 §2 rule 7). Emit it every time you know it.
- `sha256` in the response is optional. If supplied, it must match the
  canonical staged-tree hash **independently recomputed by the core**;
  a disagreement fails. Omit it unless your canonical-tree implementation
  matches the core's contract. It is not the downloaded archive's digest.

## 2. The invariants you must hold

1. **Never the plugin's word.** The core checks staged bytes against the
   lockfile before publication. This is integrity checking, not confinement:
   plugins are native executables with operator privileges, unlike the
   sandboxed frontend. Do not execute an untrusted plugin.
2. **Reproducibility: same pin → same tree hash, on any machine.** This is
   the one most fetchers break. Absolute paths embedded in the payload
   (pixi's conda-meta was the canonical bug), timestamps, ordering —
   anything environment-derived poisons the hash. Exclude bookkeeping
   metadata or normalize it before you stage.
3. **Death is not silent.** If you cannot produce a response, exit nonzero
   with useful stderr — the host reports a bounded tail. Prefer an error
   diagnostic with a source label. A response never overrides a nonzero
   exit, budget violation or cleanup failure.
4. **One bounded invocation.** Capability negotiation (itself capped at
   30s), declared-domain token waits and the fetch exchange share a 600s
   deadline including cleanup. Progress does not extend it. Declare finite
   `N/s`, `N/min` or `N/hr` rates with capacity `N >= 1`; operator
   `[throttle]` overrides take precedence.
5. **stderr is a log, not a channel.** Both output streams are drained with
   independent 16 MiB totals; stderr retains at most 64 KiB. NDJSON lines
   are capped at 1 MiB and input at 4 MiB. Exceeding a cap fails the exchange.

## 3. The pinning story, per ecosystem

The lockfile entry is `{url, version, sha256}`. Your job: make `locked`
meaningful for your transport.

- **Internal registry (artifactory/nexus/custom):** resolve to an exact
  artifact version + its registry-recorded hash on first fetch; on locked
  fetch, download *that exact artifact* and let the core's hash gate do
  the rest. Record registry/mirror/identity in `provenance`.
- **apt/dnf:** the `.deb`'s sha256 from the `Packages` index (or a dated
  repo snapshot for full reproducibility) is your pin. Extract with
  `dpkg-deb -x` into `dest_dir`. Declare the dependency closure in args —
  gripsack is not a solver. **Never run maintainer scripts** (postinst):
  config modules own system state, deterministically. FHS payloads
  (`usr/bin/...`) deploy fine; hardcoded config paths are your pour to
  rewrite before staging.
- **git:** a commit sha. A branch/tag is a *floating* ref — resolve it to
  a sha on first fetch, pin the sha.

## 4. Conformance (required before you call it done)

`pip install gripfetch-conformance`, then:

```bash
gripfetch-conformance /path/to/gripfetch-<name>
```

The suite drives your plugin exactly like the core does and asserts the
contract: request shape, NDJSON message shapes, codespacing, severity
handling, `locked` present vs absent, provenance recorded, byte-identical
tree hashes across two runs, >64KB stderr without a hang, and
death-without-response behavior. A conformance failure is a bug in the
plugin, not an opinion.

Also dogfood it for real: a module with `pluginFetch("<name>", ...)` and
a `path =` registration in `env.toml`, `grip apply` twice — second apply
must say "already satisfied" with one store path (0008 §3; finding C
proved this bites plugin fetchers).
