---
name: gripsack-debug
description: Diagnose a failing gripsack run from its structured run log — run ids, JSONL events, span causality, diagnostic codes
---

# Debugging a gripsack run

Logged `grip` commands emit structured engine events. Parsing exits and a
failure to initialize tracing can leave no log.

## 1. Find the run

```
~/.local/share/gripsack/runs/<run-id>.jsonl     # one file per run
~/.local/share/gripsack/runs/latest             # symlink to the newest
```

(`$GRIPSACK_HOME` if set, else `$XDG_DATA_HOME/gripsack`, else
`~/.local/share/gripsack`.) Start with the named run file if available:
`latest` points to the most recently initialized log, not the most recent
failure. Another command can replace it.

`runs/` is owner-only (0700), new logs are 0600. At start and normal exit,
Gripsack keeps the newest 100 completed logs by start time, plus active runs
and `latest` if outside that set. A crash releases the writer lock; the next
pass can reclaim the log. Preserve a needed log privately outside `runs/`
before starting more commands. Pre-0.57 files remain behind the tightened
directory without changes to their own modes or contents. Run-log pruning
does not prune task-invocation receipts.

## 2. Read the JSONL

One JSON object per line. Key fields:

| field | meaning |
|---|---|
| `timestamp` | RFC 3339, UTC |
| `level` | ERROR / WARN / INFO / DEBUG |
| `fields.message` | what happened |
| `fields.code` | stable diagnostic code (E101, …) when applicable |
| `spans` | the ancestry chain — this IS causality |

**Causality**: `spans` lists nesting root-first. Follow the actual recorded
operation and output names rather than inferring execution from declaration
order. To answer "why did X happen", walk up the chain; to answer "what did X
cause", filter events containing the same recorded span.

Useful filters:

```bash
home=${GRIPSACK_HOME:-"${XDG_DATA_HOME:-"$HOME/.local/share"}/gripsack"}
L="$home/runs/latest"
jq -c 'select(.level == "ERROR")' "$L"          # every error
jq -c 'select(.fields.code != null)' "$L"       # every coded diagnostic
jq -c 'select(.spans != null) | {spans, fields}' "$L"  # recorded operation ancestry
```

## 3. Diagnostic codes (the sema contract)

| code | meaning | first move |
|---|---|---|
| E000 | malformed IR JSON | frontend bug — check the emitter |
| E100 | wrong `ir_version` | core/frontend version skew — `grip doctor` |
| E130 | invalid workspace declaration | inspect the labeled declaration and the suggested field/reference fix |
| E112 | probe set unstable | restructure the workspace so probe requests converge |
| E113 | unsupported probe kind | probes are a closed enum: `executable`, `file_exists` |


## 4. The debugging loop

1. `latest` run → find ERROR events → note `code` and the span chain.
2. The rendered console error (same information, human form) includes a
   source snippet when the file is reachable — point the user at the
   exact workspace declaration line.
3. `grip plan --ir <file>` reproduces admission failures without touching
   deployed state; use an IR9 workspace envelope.
4. For more detail, re-run with `GRIPSACK_LOG=debug`.

## 5. What NOT to do

- Don't suggest editing files under `$GRIPSACK_HOME` — store paths and
  generations are immutable by contract; fix the workspace, re-apply.
- Don't suggest deleting `runs/` logs mid-investigation; they are the
  evidence.
- Don't retry on E2xx hash mismatches — that's a tampering signal
  ([RFC 0002](https://github.com/gripsack-dev/gripsack/blob/main/docs/rfcs/2026-08-22-sourcing.md) §4), escalate instead.
