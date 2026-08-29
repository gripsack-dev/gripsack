# 0015 — `grip adopt`: reversible adoption as a first-class flow

- Status: draft
- Date: 2026-08-29
- Amends: 0001 §3.7 (take-over gains prior state), 0006 (the migration
  path becomes one command)

## 1. Why

The roadmap's own sentence: *migration cost, not fetchers, is the
enemy.* Today the adoption path is manual — copy the config into the
repo, write a module, apply with `--take-over` (a global flag that
also clobbers unrelated drift), and accept that rollback **removes**
the destination instead of returning your original file. The agent
skill (`gripsack-adopt`) proves the knowledge is codifiable; the CLI
is the canonical path.

The bar (an external reviewer's phrasing, adopted as acceptance):

```
$ grip adopt ~/.config/helix     # inspects, recommends, touches nothing
$ grip rollback                  # your original files have been restored
```

## 2. The command

```
grip adopt <path> [--name <module>] [--mode owned|tracked_copy|merge]
           [--host <host>] [--yes]
```

Five phases, the middle three observable before anything writes:

1. **Inspect** — enumerate the path (dir tree or single file), sizes,
   foreign symlinks (stow/chezmoi links: read the target, note the
   origin). Refuse loudly when: the path doesn't exist, it's already
   managed (any generation manifest covers a destination), or it's
   outside `$HOME`.
2. **Recommend** — an ownership mode with a stated reason:
   - known self-rewriting app (curated table: zed, Code, discord,
     …) → `tracked_copy` ("<tool> rewrites its own config")
   - shared shell files (`.bashrc`, `.zshrc`, `.profile`,
     `.bash_profile`) → `merge` ("other tools write this file too")
   - everything else → `owned` ("<tool> doesn't rewrite its config")
   `--mode` overrides; the reason is always printed.
3. **Generate** — write into the env repo, never touching the live
   destination:
   - dir → `configs/<name>/` (verbatim copy), `modules/<name>.ts`
     using `tree(...)` with the chosen mode;
   - single file → `configs/<name>/<basename>`, single entry;
   - merge → payload is the current file's full content as the managed
     block;
   - `hosts/<host>.ts` gains the import and the modules-array entry —
     programmatically, with a conservative edit (last import +
     `modules: [` anchor); if the file doesn't match the expected
     shape, print the exact snippet instead of guessing.
4. **Plan** — run the normal plan machinery (sandboxed eval, diff vs
   the live generation) and show it, plus one extra line per
   destination: `prior state will be recorded — rollback restores it`.
5. **Confirm & apply** — `[y/N]` on a TTY, `--yes` otherwise required.
   Apply runs with **scoped take-over** (§3) and prior-state capture
   (§4).

`grip adopt` evals, so the trust gate (0013 D7) applies as usual.

## 3. Scoped take-over

`--take-over` today is global: it also clobbers *unrelated* drifted
destinations in the same apply — unacceptable inside adopt. `Ctx`
gains `take_over_entries: Option<BTreeSet<String>>`; a destination is
taken over when the global flag is set OR its `to` is in the set.
Adopt passes exactly the destinations it generated. Drift elsewhere is
never touched.

## 4. Prior state — "your original files have been restored"

Any take-over (adopt or `--take-over`) records what the destination
was **before** gripsack wrote it:

```rust
pub struct Prior {
    pub kind: PriorKind,        // File | Symlink | Absent
    pub content: Option<String>, // File: sha256 of stored bytes;
                                 // Symlink: the link target
}
```

- Real-file bytes go to a content-addressed prior blob store,
  `$GRIPSACK_HOME/prior/<sha256>` (configs are small; dedup is free).
  Generations reference the hash in their manifests, so `gc` collects
  exactly the blobs no manifest references.
- `DeployedEntry` gains `prior: Option<Prior>` (serde-defaulted; old
  manifests read fine).
- **Rollback**: the removal branch (entry in current generation,
  absent from target) restores the prior instead of deleting —
  file bytes are rewritten from the blob, a symlink is re-created,
  `Absent` falls back to today's drift-guarded removal.
- **Prune-on-undeclare**: same rule. Deleting an adopted module from
  the repo returns the machine to the pre-adopt file, not to a void.
- **Drift guard**: prior restore only fires when the destination still
  matches what gripsack deployed (`entry.hash`). If the user edited
  after adopting, the file is theirs — kept, with a warning. Merge
  entries need no prior: block-stripping already restores the foreign
  file.

This closes the last overclaim on the homepage: adoption becomes
*fully* reversible, not just "we won't clobber on the way in."

## 5. Non-goals (v1)

- Binary/source adoption (the skill's fetcher interview stays agent
  territory; `grip adopt` takes over *config* paths).
- Multiple paths per invocation; `--mode merge` for dirs.
- Converting existing foreign symlink *chains* into store payloads
  beyond reading their targets.

## 6. Acceptance (e2e)

- `grip adopt ~/.config/demo` on a fixture: repo gains
  `configs/demo/`, `modules/demo.ts`, host entry; apply manages the
  destination; **rollback restores the original real files**.
- Known-mutating app dir (`~/.config/zed`) → generated module uses
  `tracked_copy`; `~/.bashrc` → `merge`.
- Already-managed path → refusal naming the owning module.
- Scoped take-over: pre-existing drift in an unrelated module is
  preserved through the adopt apply.
- Post-adopt user edit → rollback keeps the user's file (drift guard).

## 7. Amendment — the ownership question is asked, not guessed

A hostile re-read of the shipped command found nine issues. The
philosophical one frames the rest: **adopt was being clever where it
should have been honest.** Every fix below follows one rule — when the
system can't know, it asks; when it writes, it says exactly what it
wrote.

### S1 — The heuristic tables are deleted (the smell that started this)

`SELF_REWRITING` and `SHARED_SHELL_FILES` presented folk knowledge as
detection ("helix doesn't rewrite its config" — a claim no measurement
backed). No comparable tool maintains such a table: chezmoi sets
attributes explicitly or *prompts* (`promptChoice`, defaults,
TTY-gated); debconf made ask-with-defaults-and-preseed the model 25
years ago. Adopt now **asks**, with the semantics laid out, arrow-key
select:

```
how should gripsack own these files?
  > owned        — read-only symlink into the store; the repo is the
                   only editor. For tools that never write their config.
    tracked_copy — a real file, hash-recorded; the app may rewrite it
                   and your edits are detected, never clobbered. Safe.
    merge        — one managed block inside a file other tools write.
```

The default is deliberately `tracked_copy`, not `owned`: a wrong
tracked_copy costs elegance; a wrong owned lets an app write through
the symlink into the store (the asymmetric failure tail). `owned` is
an informed opt-in. Non-interactive (`--yes`, no TTY) takes the safe
default with a loud note; `--mode` is the preseed. The generated
module file IS the persisted answer (chezmoi's promptChoiceOnce for
free).

### S2 — Directory symlinks are no longer followed

`walkdir` used `is_dir()`, which follows links: a symlink inside the
adopted tree could pull an arbitrary directory tree into the user's
repo. Directory symlinks are now skipped and listed with their
targets; broken file symlinks are skipped with a warning.

### S3 — Paths outside $HOME are refused (plan compliance)

Section 2 said it; the code didn't. Adopting `/etc/...` writes
absolute, non-portable destinations into the repo. Now refused with
the reason.

### S4 — The repo is never clobbered either

`configs/<name>/` and `modules/<name>.ts` were written unconditionally.
The tool that refuses to clobber `~/.config` must refuse to clobber
the repo: both are errors now (name the conflict).

### S5 — Honest failure messages

The eval-failure path claimed "the repo is untouched otherwise" while
payload, module, and host edit were already written. The message now
lists exactly what was written and the exact revert steps.

### S6 — Plan/output contradiction

The plan rendered "needs --take-over" for the very destinations adopt
then took over. Adopted destinations are labeled "adopt (prior
recorded)" instead.

### S7 — `update_host` is a pure, unit-tested function

Untested string surgery on user code. Extracted, with the happy-path
matrix pinned by unit tests (single-line array, empty array, multiline
array, import variants) and a strict bail-to-snippet fallback.

### S8 — Size awareness

Adopting a tree blindly copied any size into the repo. Totals were
always shown; now >25MB also warns and names the largest entries
(size is evidence, not a heuristic).

### S9 — Split into components

`commands/adopt.rs` (500 lines, five phases) becomes
`commands/adopt/{mod,inspect,generate,prompt}.rs` — pure functions at
the edges, side effects named.

### Deferred to Next

- **Read-only store payloads** (`chmod a-w` on publish): the
  structural fix for the write-through-symlink failure tail — an app
  rewriting an owned config gets EACCES instead of silently corrupting
  the store. Nix's store is read-only for the same reason.
