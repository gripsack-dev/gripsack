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
