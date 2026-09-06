# 0036 - Representation audit: every domain type, walked

Status: **done (owner exercise, post-0035)**. The rule (owner):
representations are tied (newtypes/enums over naked strings and
positional args) unless a string is genuinely clearer. This doc is
the walk's result: what got typed, what deliberately stayed, and why.

## Typed (landed 0033–0035)

| Domain | Type | Why it earns it |
|---|---|---|
| Content hashes | `PayloadHash` / `BytesHash` / `FileIdentity` | three domains that actually mixed up in production (merge journal aborts, verify false-corrupts) — now a cross-domain compare doesn't compile |
| Manifest identity field | `ManifestHash` | modal by ownership mode; constructible only from the typed producers |
| Journal boundary | `ObjectIdentity` (File\|Link), `Intended` (Removed\|Object) | the REMOVED sentinel string convention is gone from production flow |
| Recovery output | `RecoveryNote` (severity + message) | callers render by level instead of parsing text |
| Manifest paths | `DeployedEntry.from`/`key: PathBuf` | path semantics; `to` stays String — a declaration spelling (`~/.x`), NOT a path, and typing it as one would lie |
| Priors | `Prior` enums (journal + manifest) | per-variant facts, no `content: Option` soup |
| Run inputs | `ModuleInputs`, `DestView`, `RecoveryFacts` | the nine-positional and five-positional signatures are gone |
| Take-over scope | `Ctx::take_over` + `take_over_entries` (bool + scoped set) | global vs scoped absorb are different facts, visibly |

## Deliberately NOT typed (the pushback half)

| Domain | Why a newtype would hurt |
|---|---|
| Generation numbers (`u64`) | the ONLY multi-u64 confusion point was the classifier — fixed by the `RecoveryFacts` struct. Every other signature takes home-path + one u64; a `GenerationId(u64)` would tax ~40 call sites for zero mixup risk. Revisit if a second u64 domain ever shares a signature with generations. |
| Module/step/host names (`String`) | labels and map keys; there is no second name domain they can be confused with. |
| Wire fields (journal JSON, manifest JSON, lockfile, plugin NDJSON) | DTOs stay strings at the serde boundary by design; the TYPES guard construction and the in-memory flow. |
| Plan/report text | display strings, one construction site each — newtyping them is ceremony. |

## The pattern that emerged (now the convention)

1. Wire formats stay plain (serde-transparent newtypes where the wire
   is a string — zero migration cost).
2. Producers are typed; a wrong-domain value can't be BUILT.
3. Comparisons at boundaries go through `.as_str()` explicitly — the
   seam is visible, not hidden by PartialEq tricks.
4. Pure decision functions the model drives stay stringly (the
   harness enumerates abstract strings); the types guard everything
   that builds or consumes those strings in production.

## Follow-ups

- The kill-point matrix (0025) + persistence evidence (0035 deferral)
  are the remaining places where a typed boundary matters more than
  a type.
