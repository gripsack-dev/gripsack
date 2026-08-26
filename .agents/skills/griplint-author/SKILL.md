---
name: griplint-author
description: Author a config linter as a data pack — research the tool's config surface, build the versioned key table in crates/griplint/packs/<tool>.toml, pin behavior with fixture tests. Use when implementing a linter issue or refreshing a pack.
---

# Authoring a config linter (data pack)

You are adding `crates/griplint/packs/<tool>.toml` to the gripsack
repo. The engine is data-driven: your job is tool knowledge and data.
The work is rigorous because the output is load-bearing — a wrong table
entry trains users to ignore the linter. **Never guess a key — every
table entry traces to documentation or the tool's source.**

Study the reference pack first: `crates/griplint/packs/helix.toml`
(plus its fixtures in `crates/griplint/fixtures/helix/`).

## 1. Research the tool's config surface (before any data)

- Read the tool's **official config documentation end to end**. Not a
  blog post, not memory — the current docs, in a browser or the tool's
  repo. Record the docs URL + version in your notes.
- Prefer machine-readable truth when it exists, in this order:
  1. an official schema (JSON schema, `--dump-config`, source enums);
  2. the tool's config parsing/defaults source;
  3. prose docs.
  Note which one you used per section — you'll cite it in the pack.
- Enumerate per section: keys, value types, allowed choices (enums),
  deprecated/renamed keys with their replacements.
- Identify **user-defined sections** — keybindings, language names,
  per-plugin tables. These are FREE_FORM; validating them is a
  false-positive factory.
- Identify **which files** the linter handles: exact basenames
  (`config.toml`), and which it deliberately ignores (e.g.
  `languages.toml` — say why in the pack header comment).
- Record the tool's current stable version prefix(es) — your
  `supported` set. Check the issue you're closing; it links the docs.

## 2. Write the pack (`crates/griplint/packs/<tool>.toml`)

```toml
[meta]
tool = "helix"
handles = ["config.toml"]
format = "toml"                # toml | yaml | json
supported = ["25."]            # tool-version prefixes these tables cover
# lenient = ["pyproject.toml"]   # optional: shared files lint leniently
series = "v25"                 # table series provenance

# Sources: https://docs.helix-editor.com/configuration.html (25.7)

[rules.""]                     # bare top-level keys
theme = { types = ["string"] }

[rules.editor]
scrolloff = { types = ["integer"] }
line-number = { types = ["string"], choices = ["absolute", "relative"] }
old-key = { types = ["boolean"], deprecated = "new-key" }

[rules.keys]
_free = true                   # user-defined sections: anything goes
```

- Types: `string | integer | boolean | float | array | table` (a list
  means any of them). Choices are case-sensitive. TOML `true` is not an
  integer; duration/size strings are `string` even when numeric-looking.
- A key-level FREE_FORM rule serializes as `"key.path" = "free"`.
- A whole section type-checked but not enumerated (enable/disable
  tables): `_rule = { types = ["table"] }`.
- Per-file tables (ruff.toml vs pyproject.toml) live under
  `[files."<basename>".rules.*]`; single-file tools use
  `[files."<basename>".rules.*]` too — the structure is uniform.
- **Cite your source per section** as a comment (docs anchor or source
  file). Reviewers must be able to check every line against a link.
- Partial coverage beats wrong coverage. Ship sections that are right;
  omit what you haven't tabulated rather than emitting false unknown-
  section errors.

## 3. Pin behavior with fixtures (required)

`crates/griplint/fixtures/<tool>/<case>/` — the config under its REAL
basename, `expected.json`, optional `tool_version`. Author
`expected.json` by running the reference implementation (or the engine
once it lands) and pasting **verified** output.

Minimum case set, every pack: `clean`, `typo-key` (did-you-mean help,
span on the key's line), `wrong-type`, `bad-choice`, `free-form`,
`unknown-section` (skip if no fixed sections), `version-stale` (the
coverage warning, linting still runs), `ignored-file`, `parse-error`.
Plus one fixture per quirk you found in research (nested dotted
sections, deprecations, inline tables).

- **Fixture files are real config files to other tools.** A fixture
  named `ruff.toml` is discovered by ruff itself, `mise.toml` by mise —
  check what CI sniffs.
- A pack NEVER errors on files it doesn't handle, unknown tool
  versions, or weird-but-legal configs. Those are the three ways a
  linter teaches users to turn it off.

## 4. Land it

- `cargo test -p griplint` — the pack loader gate (every pack must
  load; the shape tests bite).
- PR against gripsack-dev/gripsack; the diff *is* the changelog review.
- Close the linter issue (link the PR); if it was labeled
  `good first issue`, write the PR description assuming the next
  contributor learns the repo from it.

## External plugins (exotic formats)

RON, KDL, and custom formats stay out-of-tree executables speaking the
NDJSON protocol — the data-pack path covers TOML/YAML/JSON only. Write
those against
[griplint-conformance](https://github.com/gripsack-dev/griplint-conformance);
the suite drives your plugin exactly like the core does.
