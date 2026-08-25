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

- `args` is opaque to the core — your module's `plugin_fetch("<name>", args)`
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
- `sha256` in the response is advisory and ignored: **the core recomputes
  identity from the staged tree**. Never spend effort making your hash
  match anything; spend it making the tree right.

## 2. The invariants you must hold

1. **Never the plugin's word.** Your output is untrusted by design. If you
   are wrong or malicious, the worst outcome is a failed apply, never a
   poisoned store — the core hash-checks against the lockfile before
   anything enters it. Don't optimize around this; lean on it.
2. **Reproducibility: same pin → same tree hash, on any machine.** This is
   the one most fetchers break. Absolute paths embedded in the payload
   (pixi's conda-meta was the canonical bug), timestamps, ordering —
   anything environment-derived poisons the hash. Exclude bookkeeping
   metadata or normalize it before you stage.
3. **Death is not silent.** If you cannot produce a response, exit nonzero
   with a useful stderr tail — the core synthesizes `gripfetch-<name>/E02`
   with that tail attached. Better: emit an error diagnostic and a
   response, then exit nonzero anyway.
4. **No unbounded waits.** The exchange has a 600s deadline; a stuck
   plugin is killed and reported as a failure. Long downloads are fine —
   emit `progress` to stay visibly alive.
5. **stderr is a log, not a channel.** The core drains it concurrently
   (any volume is safe) and shows its tail only on protocol death.

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

Also dogfood it for real: a module with `plugin_fetch("<name>", ...)` and
a `path =` registration in `env.toml`, `grip apply` twice — second apply
must say "already satisfied" with one store path (0008 §3; finding C
proved this bites plugin fetchers).
