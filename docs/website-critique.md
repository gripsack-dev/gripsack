# Website and README critique (2026-11-08)

A first-impression and structure review of gripsack.dev, the README and the
user docs, written before the refresh landed. Kept as the record of what
was wrong and why the pages now look the way they do.

## Thirty-second test on the landing page

- **What is it?** Not answerable from the hero. "A declared workspace. A
  little more control." and "Pack what you need. Know what you manage." are
  mood lines, not a description. The first concrete noun a visitor meets is
  "TypeScript" in sentence three.
- **Why care?** The actual differentiator (apply is a transaction: review a
  plan, keep every generation, roll back in one command, and the core
  decisions are machine-proved) is a footnote in a grey "note" box.
- **How do I start?** The quick start is fine, but the install call to
  action points at a GitHub tag page instead of the install guide, and
  nothing shows the tool running although sixty demo GIFs exist in `demos/`
  and `img/`.
- **No picture anywhere.** Zero diagrams on the whole site. Concepts that
  beg for one (generations and rollback, ownership and drift, what a crash
  does) are explained in dense prose.

## Narrative arc

The docs are feature silos with no arc. There is no page that says
"problem, idea, how it works, why it is safe, start here". `architecture.md`
reads as an internal interface contract ("Admit: the Rust core validates the
resulting intermediate representation"); `safety.md` reads as a defence
against a hostile reviewer, not a guide for a person deciding whether to
trust the tool with their dotfiles.

## Information architecture

- Twenty flat rail entries with no grouping; labels like "native artifact
  sharing", "agent guidance", "retired source builder" mean nothing to a
  newcomer. The changelog page contains no changes.
- Topbar says "Documentation ↗" for an internal link; the hero's primary
  button says "Get gripsack ↗" and scrolls down the page.

## Verbosity and review-room jargon

Every page carries three to six disclaimers, many repeated across pages
("profile rollback does not roll back database contents" appears on five
pages). Internal vocabulary leaks everywhere: *admitted*, *admission*,
*realization*, *frozen consumers*, *evidence*, *qualification*, *receipt*,
*authority*, *lane*, *calibration*, *lifecycle admission*, plan IDs, and
sentences such as "not evidence that those checks ran for this site" or
"obtained during release preparation; that alone is not evidence of a
Gripsack runtime test". The README is ~430 lines, of which ~250 are the
native-sharing contract (SCC lowering, inode pins, receipt maps): an
engineering ledger, not a README.

## Stale and inconsistent content

- `workspaces.md` still lists `recipe` and `image` outputs; both were
  retired in 0.51. The README's documentation list links "BuildKit and OCI
  images". Homebrew/BuildKit/mac remnants survive as "removed rather than
  replaced" asides on several pages.
- `architecture.md` and `safety.md` link the guarantee ledger, which plan
  0056 froze; the proofs that replaced it are not mentioned anywhere.
- README install section is prose; the site has a script. README and site
  describe sharing at wildly different depths.

## Accessibility and mobile

The base is sound (skip link, `:focus-visible`, reduced-motion, 44px
targets, responsive grids). Weak spots: 10–11px mono text in several
places, `display: block` tables without a caption, hero card rotated by
1deg (fine) and no `alt` on decorative-but-informative inventory text.

## What changed in response

Landing page rewritten around the story (declare → plan → apply → keep a
way back → proved core) with a demo recording, a CSS-animated generation
strip and a diagram; rail grouped into Start / Declare / Operate / Trust /
About; new "How gripsack stays safe" page with the rings, what is proved
today, what is assumed, and what is not claimed; four d2 diagrams; safety,
architecture, compare, changelog and workspaces rewritten; jargon sweep
over the remaining pages; README cut to a story with the sharing contract
moved to `docs/sharing-reference.md`.
