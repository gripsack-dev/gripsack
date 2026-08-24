"""Tree entries — directory-shaped config deploys via eval-time
expansion (0006 critique; the IR stays per-file).

>>> from gripsack import module, tree  # doctest: +SKIP
>>> module("zed", config={**tree("configs/zed", "~/.config/zed")})  # doctest: +SKIP

Adding or removing files in the directory is picked up at the next
eval; files dropped from the tree are pruned at apply (0008 §3).
"""

from __future__ import annotations

from pathlib import Path

from .entries import Dest, Ownership


def tree(src: str, to: str, mode: Ownership = Ownership.TRACKED_COPY) -> dict[str, Dest]:
    """Expand a directory into per-file config entries: every file under
    `src` maps to its mirror under `to`, with `mode`.

    Merge the result into a module's config: `config={**tree(...)}`.
    """
    root = Path(src)
    entries: dict[str, Dest] = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and not path.is_symlink():
            rel = path.relative_to(root).as_posix()
            entries[f"{src}/{rel}"] = Dest(f"{to}/{rel}", mode)
    return entries
