"""Extraction from the website source: every published TypeScript
block in doc/**/*.md, plus exhaustive pairing against the manifest —
any unclassified block, or any manifest entry without its block, is
drift and fails loudly."""

from __future__ import annotations

import html
import re
from dataclasses import dataclass
from pathlib import Path


class CheckFailure(Exception):
    pass


@dataclass
class Block:
    """One extracted TypeScript block from the site docs."""

    file: str          # site-repo-relative doc path
    line: int          # 1-based line of the code inside the md file
    title: str | None  # window title (window blocks only)
    fenced: bool
    code: str


def extract_blocks(site: Path) -> list[Block]:
    """Every TypeScript block in doc/**/*.md, in document order.

    Two published spellings: fenced ```ts / ```typescript blocks, and
    the site's <div class="window"> code cards (language-typescript).
    """
    blocks: list[Block] = []
    for md in sorted((site / "doc").rglob("*.md")):
        rel = md.relative_to(site).as_posix()
        text = md.read_text(encoding="utf-8")
        lines = text.split("\n")

        # fenced blocks: a small scanner so plain ``` fences (output
        # samples etc.) never disturb ts-fence boundaries or numbering
        i = 0
        while i < len(lines):
            stripped = lines[i].lstrip()
            if stripped.startswith("```") and stripped[3:].strip() in ("ts", "typescript"):
                start = i + 1
                body: list[str] = []
                j = start
                while j < len(lines) and not lines[j].lstrip().startswith("```"):
                    body.append(lines[j])
                    j += 1
                if j >= len(lines):
                    raise CheckFailure(f"{rel}:{start}: unterminated ts fence")
                blocks.append(Block(rel, start + 1, None, True, "\n".join(body)))
                i = j + 1
            else:
                i += 1

        # window cards: capture the code, inherit the nearest title
        for m in re.finditer(
            r'<pre><code class="language-typescript">(.*?)</code></pre>',
            text,
            re.DOTALL,
        ):
            title = None
            t = text.rfind('class="wtitle">', 0, m.start())
            if t != -1:
                content = text[t + len('class="wtitle">'):]
                title = html.unescape(content[:content.find("<")])
            code = html.unescape(m.group(1))
            blocks.append(Block(rel, text.count("\n", 0, m.start()) + 1, title, False, code))

        blocks.sort(key=lambda b: b.line)  # document order across kinds
    return blocks


def pair_with_manifest(blocks: list[Block], examples: list[dict]) -> dict[str, Block]:
    """Manifest id → extracted block. Every block must be claimed by
    exactly one entry and every entry must find its block — anything
    else is unclassified drift and fails loudly."""
    by_id: dict[str, Block] = {}
    unclaimed = list(blocks)
    problems: list[str] = []
    for entry in examples:
        want_file = f"doc/{entry['file']}"
        loc = entry["locator"]
        found = None
        fenced_same_file = [
            x for x in unclaimed if x.file == want_file and x.fenced
        ]
        for b in unclaimed:
            if b.file != want_file:
                continue
            if "window" in loc:
                if not b.fenced and b.title == loc["window"]:
                    found = b
                    break
            elif b.fenced and fenced_same_file.index(b) == loc["fence"]:
                found = b
                break
        if found is None:
            problems.append(
                f"manifest entry {entry['id']!r}: no matching doc block at "
                f"{want_file} ({loc}) — example removed or moved? re-classify"
            )
            continue
        unclaimed.remove(found)
        by_id[entry["id"]] = found
    for b in unclaimed:
        first = b.code.strip().split("\n")[0][:72]
        problems.append(
            f"unclassified TypeScript example at {b.file}:{b.line}"
            f"{' (%r)' % b.title if b.title else ''}: {first!r} — "
            "add it to the manifest (runnable with fixtures, or an "
            "explicitly classified fragment)"
        )
    if problems:
        raise CheckFailure("extraction drift:\n  " + "\n  ".join(problems))
    return by_id
