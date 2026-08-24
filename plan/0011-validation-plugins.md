# 0011 — Validation plugins (linters) at eval

- Status: draft
- Date: 2026-08-24
- Amends: 0005 §5 (frontend protocol), 0009 §5 (validator plugins)

## 1. The north star

0009's honesty sentence: *a misconfiguration in any file gripsack
touches should produce the same quality of error as a typo in a
module.* Deployed config files are the gap. A typo'd key in
`yazi.toml` today deploys cleanly and fails at tool runtime, in a
different terminal, an hour later. Linters close that gap — at eval,
before anything stages.

## 2. Why eval-time

- A lint failure is an **authoring error**, the same class as E109. It
  should render as a coded diagnostic with a span, before any file is
  staged — at apply time the bad file is already written (for owned
  mode: already symlinked over the user's config).
- **Zero IR surface.** Linting happens in the frontend before
  emission; the opt-in never enters the IR. No schema change, no
  `ir_version` bump, no three-sides PR.
- The LSP endgame (0009 §5) is preserved: validator diagnostics are
  the same shape as sema and config-parser diagnostics, exercised on
  every `grip apply` from day one.

v1 lints static files only. `template()` entries render at activation,
so eval-time lint would see the template, not the output — post-render
linting is a check-time concern, deferred.

## 3. The mechanism

`griplint-<tool>` plugins, speaking the 0009 §2 NDJSON envelope,
invoked **by the frontend during evaluation**. One request per
module–linter pair:

```json
{"op": "lint", "paths": ["configs/yazi/yazi.toml", "configs/yazi/keymap.toml"],
 "tool_version": "25.5.31"}
```

- `paths` is the module's config source set, post-`tree()` expansion —
  the frontend already knows it; repo files are linted in place.
- `tool_version` comes from the host lockfile pin — the lock that pins
  the binary also pins the config schema. Unpinned → the linter uses
  its latest schema and emits a warning.

## 4. Convention over configuration

The linter owns the tool's layout knowledge. It dispatches on
basename, walks what it understands, and **ignores what it doesn't** —
one `griplint-yazi` covers `yazi.toml`/`keymap.toml`/`theme.toml`, and
a `LICENSE` caught in a `tree()` is not its business. Unknown files
are never errors; a linter that chokes on a layout it doesn't
recognize is a bad linter, not a bad config. No per-entry file lists
in module declarations: `module("yazi", ..., lint="yazi")` is the
entire opt-in.

## 5. The eval envelope (amends 0005 §5)

The eval subprocess's stdout becomes:

```json
{"ir": {...}, "diagnostics": [...]}
```

This is the eval **wire protocol**, not the IR: `schema/ir/`,
`ir_version`, and the IR document are untouched. Core and frontend
versions are pinned together (doctor enforces the match), so the
cutover needs no fallback parsing. Any error-severity diagnostic fails
eval; warnings flow and render without failing (0009 §2.4). Exit
non-zero with no parseable envelope remains a frontend traceback —
developer bugs, passed through untouched as today.

## 6. One renderer

0009 §2.1 holds across this boundary too: plugins and frontends
**serialize** diagnostics; only the core **renders** them — same
snippet, colors, and codespacing (`griplint-yazi/A01`; core `E0xx`
reserved) as every other diagnostic. Labels point into the config file
(span domain b); the frontend attaches the module-callsite label from
the provenance it already captures (span domain a). There is no second
renderer and there never will be.

## 7. Registration and provisioning

Per 0010: `[linters.<name>]` in env.toml, provisioned into the
frontend venv from a pinned package, or a `path` override for linter
development. `lint = "<name>"` must resolve against the registry —
an unregistered name is a hard eval error with the module-line span.
No PATH-discovery fallback: discovery is how configs end up silently
unlinted on one machine and linted on another.

## 8. Linters are not verify

Linters are **static shape** (does this key exist in yazi 25.x);
`verify_*` is **runtime smoke** (does the deployed thing work). A
"native lint" mode that shells out to the tool itself (`hx --health`)
belongs to the verify side, not here — keep the two apart.

## 9. Implementation map

- `gripsack-config` — tolerate and parse `[linters]` (env.toml is
  `deny_unknown_fields`, 0009 §3); surface in doctor.
- `frontend.rs` — linter packages join `[eval] deps` at provisioning
  (mechanism already generic).
- python frontend — `Diagnostic` dataclass matching the serde shape,
  an NDJSON plugin host (~50 lines), the `lint=` hook in module
  evaluation, env.toml registry read.
- core `eval.rs` — parse the envelope, render `diagnostics`, proceed
  with `ir` (~30 lines, reuses `render_diagnostics`).
- `griplint-*` repos — plain Python, vendored schemas per tool
  version, console scripts per 0010.

*The contract is the diagnostic shape; everything else — who invokes,
when, from where — is allowed to evolve.*
