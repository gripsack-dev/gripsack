# 0009 — Diagnostics across boundaries

- Status: draft
- Date: 2026-08-23
- Amends: 0002 §4 (plugin protocol), 0004 §3 (diagnostics)

## 1. The problem

Frontend-land diagnostics are beautiful: spans, codes, snippets. The
moment work crosses a process boundary — a fetcher plugin, a builder
plugin, a validator — that beauty currently stops at a nonzero exit
code and a stderr blob. This doc aligns on what we require from every
pluggable component so it never does.

## 2. One protocol for every pluggable thing

Fetchers (0002), builders (later), and validators (§5) all speak the
same envelope: NDJSON messages on stdout. Three message types:

```json
{"type": "response", "id": 1, "result": {...}}
{"type": "diagnostic", "diagnostic": {...}}
{"type": "progress", "current": 1048576, "total": null}
```

Requirements for plugin authors:

1. **Structured diagnostics, not stderr prose.** A diagnostic is our
   shared shape: `code`, `severity`, `message`, `labels[]` (span +
   note), `help`. The core renders it identically to its own —
   snippet, colors, all of it.
2. **Codespacing.** Plugin codes are prefixed with the plugin name:
   `gripfetch-artifactory/A01`. Core codes (`E0xx`…) are reserved.
3. **Span domains.** A label's span may point at: (a) the user's
   module (the invocation carries the calling span, so a plugin can
   blame the exact `plugin_fetch(...)` line), (b) a plugin-owned file
   (its own config), or (c) nothing — context in the message. All
   three render.
4. **Severity is not exit status.** Warnings flow without failing;
   an error diagnostic ends the step. Exit 0 = success regardless.
5. **Death is not silent.** Nonzero exit with no structured diagnostic
   → the core synthesizes one (`E2xx plugin died`, stderr tail
   captured as the note). Even a badly written plugin renders
   decently.
6. **Verification is orthogonal.** Hash-checking of returned bytes
   (0002 §4) happens no matter how friendly the diagnostics are.
7. **Everything lands in the run log** (0004 + trace): plugin events
   hang off the span chain `run → apply → module → step → plugin`, so
   the JSONL causality story survives the boundary too.

## 3. Our own config files are first-class

`env.toml` and the user config are parsed with the same diagnostics:

- unknown keys are **errors** (`deny_unknown_fields`), with
  "did you mean" help from the known-key list;
- toml parse errors map byte spans to line:col and render with a
  snippet of the actual config line;
- config diagnostics are `E4xx`.

A mistyped `keep_generations = "twnety"` should point at the exact
line in `env.toml` with the same care as a module error. Anything less
would betray the standard we set for frontends.

## 4. Serializable diagnostics

`Diagnostic`, `Severity`, `Label`, `Span` derive serde — diagnostics
are data on the wire (plugin protocol), on disk (run logs), and
eventually in the editor (LSP). One shape, four transports: console,
JSONL, plugin NDJSON, LSP.

## 5. The north star: validation plugins and the editor

V1.0, not current scope — but the contract is designed so nothing
needs redesign when we get there:

- **Validator plugins** (`grip check`, later): same protocol, no
  fetch. A validator for kitty config receives the file, emits
  diagnostics with spans into it. Modules declare validators per
  config entry (opt-in).
- **The LSP** (0004 §5) composes three existing pieces: sema passes
  over IR (module-level errors), the config parser (env.toml errors),
  and validator plugins (per-file errors like kitty's). All three emit
  the same span-labeled diagnostics; the LSP maps them into the
  editor. The work is the shim, not the analysis.

The sentence that keeps us honest: *a misconfiguration in any file
gripsack touches should produce the same quality of error as a typo in
a module.*
