# 0006 — Gradual migration and coexistence

- Status: draft
- Date: 2026-08-22
- Amends: 0001 §3.1 (module shape — `source` becomes optional)

## 1. The question

"I'm on macOS with a working brew setup. I don't want to migrate my
packages — I want gripsack to manage my dotfiles. Is that possible?"

Yes — that is a first-class usage level, not a degenerate case. The one
change it required: **`source` is optional on a module.** A module with
no source has no payload to fetch; its content *is* its config files.

## 2. Adoption levels

**Level 0 — grip installed, nothing managed.** No env repo, no apply.
Costs nothing, changes nothing.

**Level 1 — dotfiles only.** Modules without sources:

```python
helix = module(
    "helix",
    config={
        "config.toml": tracked_copy("~/.config/helix/config.toml"),
        "languages.toml": tracked_copy("~/.config/helix/languages.toml"),
    },
)
```

brew/apt/mise keep managing binaries; gripsack manages `~/.config`.
You get the parts package managers never gave you: versioned configs in
a repo, drift detection, and rollback — edit a dotfile, `grip apply`,
new generation; broke something, `grip rollback`. No package ever
changes hands.

**Level 2 — cherry-picked packages.** Add sources only where gripsack
wins: a pinned tool version brew keeps upgrading, something not
packaged anywhere, a from-source build. Coexistence is PATH ordering —
gripsack's profile bin dir is one ordinary entry, and first match wins,
so you decide per tool who wins. `grip why-owns <path>` answers "who
manages this?" when you forget.

**Level 3 — full env repo.** Packages mostly via gripsack; brew for the
long tail (GUI apps, casks — gripsack is user-scoped and CLI-first).
This is the "clone repo on machine B and boom" endgame, reached
incrementally, and reversible at every step: `grip rollback`, or just
delete the profile dir and nothing was ever touched outside it.

## 3. Why the architecture already supports this

- A source-less module's payload is its own files; they hash into the
  store like anything else and deploy per ownership mode (0001 §3.7).
  Generations include config-only changes — nothing special-cased.
- `tracked-copy` exists precisely for the level-1 reality: your existing
  configs get copied into the store on first apply, then drift is
  detected (`keep / adopt / restore`).
- The DAG executor tolerates modules with no build and no runtime deps —
  they're leaf nodes that only deploy files.
- Everything is user-scoped already (0001 §2.4): no root, no daemon, no
  conflict with brew's ownership of `/opt/homebrew`.

## 4. The adopt direction (later)

Manually copying existing configs into module files works today. The
planned convenience is `grip adopt ~/.config/helix/config.toml`: copies
the live file into the env repo's module dir, writes the config entry,
ready for the first apply. Not scheduled before `apply` itself works.

## 5. macOS notes

- Facts report `os = "macos"`; platform-conditional config picks
  mac-specific files where tools split them.
- The activation adapter for services on macOS is launchd (the
  `SystemdUser` adapter's counterpart) — declared intent, adapter
  translates (0001 §3.8). Lands with activation, not before.
- brew and gripsack never fight: brew owns `/opt/homebrew`, gripsack
  owns its profile dir and the config files you hand it.

## 6. Non-goals

- Importing brew/apt state or uninstalling foreign packages. gripsack
  manages what its modules declare and touches nothing else.
- GUI app management (casks, DMGs) — out of user-scope charter.
