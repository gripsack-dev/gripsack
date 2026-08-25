"""Your first module — deploys a tiny config file, no network needed.

Try it:

    grip check          # eval + validate + lint, zero side effects
    grip apply          # build generation 1 and activate it
    ls -l ~/.config/hello/
    grip generations    # every apply is a generation; rollback is instant
"""

from gripsack import module, tree
from gripsack.entries import Ownership

# tree() maps a whole directory of config files into the store.
# Ownership.OWNED: read-only symlinks into the store — edits go
# through this repo, and git is your editor. (The tree default is
# TRACKED_COPY — real files with drift detection.)
module(
    "hello",
    config=tree("configs/hello", "~/.config/hello", mode=Ownership.OWNED),
)
