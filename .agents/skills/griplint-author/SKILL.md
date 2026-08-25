---
name: griplint-author
description: Author a griplint-* config linter for a new tool — research the tool's config surface, build versioned key tables, pin behavior with fixture tests, and release. Use when implementing a linter issue or adding packages/griplint-<tool>/.
---

# Authoring a griplint-* linter

You are adding `packages/griplint-<tool>/` to this repo. The pipeline is
built; your job is tool knowledge, data, and tests. The work is rigorous
because the output is load-bearing: a wrong table entry trains users to
ignore the linter. **Never guess a key — every table entry traces to
documentation or the tool's source.**

Study the reference implementation first: `packages/griplint-helix/`.

## 1. Research the tool's config surface (before any code)

- Read the tool's **official config documentation end to end**. Not a
  blog post, not memory — the current docs, in a browser or the tool's
  repo. Record the docs URL + version in your notes.
- Prefer machine-readable truth when it exists, in this order:
  1. an official schema (JSON schema, `--dump-config`, source enums);
  2. the tool's config parsing/defaults source;
  3. prose docs.
  Note which one you used per section — you'll cite it in the table file.
- Enumerate per section: keys, value types, allowed choices (enums),
  defaults (context only — not linted), deprecated/renamed keys with
  their replacements.
- Identify **user-defined sections** — keybindings, language names,
  per-plugin tables. These become `FREE_FORM`; validating them is a
  false-positive factory.
- Identify **which files** the linter handles: exact basenames
  (`config.toml`), and which it deliberately ignores (e.g.
  `languages.toml` — say why in linter.py).
- Record the tool's current stable version(s) — your `SUPPORTED` set.
  Check the issue you're closing; it links the docs.

## 2. Scaffold the package

Copy the reference layout, rename, adjust:

```
packages/griplint-<tool>/
├── pyproject.toml          name griplint-<tool>, version 0.1.0,
│                           [project.scripts] griplint-<tool>, dep griplint-common
├── README.md               one paragraph + link to this repo
└── src/griplint_<tool>/
    ├── __init__.py
    ├── main.py             serve_linter(LINTER) — nothing else
    ├── linter.py           LINTER = Linter(name, handles, resolve_table, checks)
    └── tables/
        ├── __init__.py     SUPPORTED prefixes + resolve() with coverage warning
        └── v<NN>.py        the key table — pure data
```

uv workspace membership is automatic (`packages/*`). `uv sync
--all-packages` puts `griplint-<tool>` on `.venv/bin`.

## 3. Write the key table (tables/v<NN>.py)

- Pure data: `Rule((types,), choices=..., deprecated=...)`, `FREE_FORM`
  for user-defined sections, `""` for bare top-level keys.
- **Cite your source per section** as a comment (docs anchor or source
  file). Reviewers must be able to check every line against a link.
- Type rules that bite: TOML `true` is not an integer (the engine
  rejects bools for int rules); duration/size strings are `str` even
  when they look numeric; choices are case-sensitive.
- Partial coverage beats wrong coverage. A table for `[editor]` and
  `[editor.cursor-shape]` that is *right* ships; mark uncovered sections
  in a comment with a "TODO(coverage)" — never emit unknown-section
  errors for sections you didn't model: if the tool has sections you
  haven't tabulated yet, omit them from the table ONLY if unknown
  sections are rare there; otherwise cover them.

## 4. Pipeline extras (checks=, only when the table can't say it)

- Cross-key constraints, semantic rules ("`wrap-at-text-width` needs
  `enable = true`"): one plain function
  `(Document, table) → Iterable[Diagnostic]`, appended to
  `Linter(checks=[...])`.
- Codes: `A0x` is the engine's. Tool checks start at `B01`.
- One check = one small function. No registries, no decorators.

## 5. Pin behavior with fixtures (required)

`tests/fixtures/<case>/` — the config file under its REAL basename,
`expected.json`, optional `tool_version`. Author `expected.json` by
running the linter and pasting **verified** output (`<input>` as the
span file). `check_fixtures(LINTER, ...)` is the gate; an empty
fixtures dir fails.

Minimum case set, every linter:

| case | proves |
|---|---|
| `clean` | a realistic valid config passes with zero diagnostics |
| `typo-key` | unknown key → `A01` error, did-you-mean help, span on the key's line |
| `wrong-type` | `A04`, and bool-for-int is caught |
| `bad-choice` | `A05` with the allowed values |
| `free-form` | user-defined sections pass anything |
| `unknown-section` | `A02` with section suggestion (skip if the tool has no fixed sections) |
| `version-stale` | coverage warning `W10`, and linting still runs |
| `ignored-file` | a file the linter doesn't handle → `[]` |
| `parse-error` | invalid TOML → `A00` with a span, not a crash |

Plus one fixture per tool-specific check and per quirk you found in
research (nested dotted sections, deprecations, inline tables).

## 6. Verify beyond pytest

```bash
uv run pytest -q                                          # everything green
printf '[bad]\nconfig = true\n' > /tmp/<tool>.toml
echo "{\"op\":\"lint\",\"paths\":[\"$PWD/<real-config>\"],\"tool_version\":null}" \
  | .venv/bin/griplint-<tool>                             # NDJSON in, diagnostics out
```

- Spans point at the key's line — open one expected.json and eyeball
  the line number against the input file.
- **Fixture files are real config files to other tools.** A fixture
  named `ruff.toml` is discovered by ruff itself, `mise.toml` by mise,
  etc. CI runs ruff with `--isolated` for exactly this reason — if you
  add a tool to CI, check what it sniffs.
- The linter NEVER errors on files it doesn't handle, unknown tool
  versions, or weird-but-legal configs. Those are the three ways a
  linter teaches users to turn it off.
- Bonus dogfood: register it in a gripsack env repo with
  `[linters.<tool>] path = "<repo>/.venv/bin/griplint-<tool>"`, add
  `lint = "<tool>"` to a module, run `grip apply` — the diagnostic
  should render with a snippet.

## 7. Release

- All tests green, ruff clean (`uvx ruff check packages`).
- Add the package row to the repo README's package table.
- PR, merge, tag `griplint-<tool>-v0.1.0`, push the tag — the release
  workflow publishes to PyPI. Tag version must equal the pyproject
  version or the workflow fails.
- Close the linter issue (link the PR); if it was labeled
  `good first issue`, write the PR description assuming the next
  contributor learns the repo from it.
