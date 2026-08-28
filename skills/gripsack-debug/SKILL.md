---
name: gripsack-debug
description: Diagnose a failing gripsack run from its structured run log — run ids, JSONL events, span causality, diagnostic codes
---

# Debugging a gripsack run

Every `grip` invocation writes a structured log an agent can read end to
end. You never need to guess what the engine did — it is written down.

## 1. Find the run

```
~/.local/share/gripsack/runs/<run-id>.jsonl     # one file per run
~/.local/share/gripsack/runs/latest             # symlink to the newest
```

(`$GRIPSACK_HOME` if set, else `$XDG_DATA_HOME/gripsack`, else
`~/.local/share/gripsack`.) If the user pastes an error, the run id is
in the console output — every event carries `run_id`. Start at
`latest` unless they name one.

## 2. Read the JSONL

One JSON object per line. Key fields:

| field | meaning |
|---|---|
| `timestamp` | RFC 3339, UTC |
| `level` | ERROR / WARN / INFO / DEBUG |
| `fields.message` | what happened |
| `fields.code` | stable diagnostic code (E101, …) when applicable |
| `spans` | the ancestry chain — this IS causality |

**Causality**: `spans` lists the nesting root-first, e.g.
`["run", "plan", "module:helix", "step:fetch"]` means "this event
happened inside the fetch step of helix, inside plan, inside the run".
To answer "why did X happen", walk up the chain; to answer "what did X
cause", grep for events whose `spans` contain X's span name.

Useful filters:

```bash
L=$(readlink ~/.local/share/gripsack/runs/latest)
jq -c 'select(.level == "ERROR")' "$L"          # every error
jq -c 'select(.fields.code != null)' "$L"       # every coded diagnostic
jq -c 'select([.spans[].name] | index("module:helix"))' "$L"  # one module's events
```

## 3. Diagnostic codes (the sema contract)

| code | meaning | first move |
|---|---|---|
| E000 | malformed IR JSON | frontend bug — check the emitter |
| E100 | wrong `ir_version` | core/frontend version skew — `grip doctor` |
| E101 | unknown module dependency | typo or missing module file; the span points at the line |
| E102 | destination not absolute/`~/` | fix the path in the module |
| E103 | module mixes `steps` with declarative fields | pick one shape |
| E104 | `needs` references unknown step | check sibling ids and `module:step` refs |
| E106 | duplicate or reserved (`done`) step id | rename the step |
| E107 | undeclared resource | `resource("name")` first, or use a built-in |
| E112 | probe set unstable — each eval round requested new probes (cap 4) | a probe depending on a probe is an authoring error; restructure the host entrypoint (0013 D6) |
| E113 | unsupported probe kind | probes are a closed enum: `executable`, `file_exists` |


## 4. The debugging loop

1. `latest` run → find ERROR events → note `code` and the span chain.
2. The rendered console error (same information, human form) includes a
   source snippet when the file is reachable — point the user at the
   exact module line.
3. `grip plan --ir <file>` reproduces sema failures without touching
   the system; `grip plan --ir <file> <module>` scopes to one module.
4. For more detail, re-run with `GRIPSACK_LOG=debug`.

## 5. What NOT to do

- Don't suggest editing files under `$GRIPSACK_HOME` — store paths and
  generations are immutable by contract; fix the module, re-apply.
- Don't suggest deleting `runs/` logs mid-investigation; they are the
  evidence.
- Don't retry on E2xx hash mismatches — that's a tampering signal
  (plan/0002 §4), escalate instead.
