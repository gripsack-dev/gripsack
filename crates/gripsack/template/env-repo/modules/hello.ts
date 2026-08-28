// Your first module — deploys a tiny config file, no network needed.
//
// Try it:
//
//     grip check          # eval + validate + lint, zero side effects
//     grip apply          # build generation 1 and activate it
//     ls -l ~/.config/hello/
//     grip generations    # every apply is a generation; rollback is instant

import { module, tree } from "@gripsack/core";

// tree() maps a whole directory of config files into the store.
// "owned": read-only symlinks into the store — edits go through this
// repo, and git is your editor. (The tree default is "tracked_copy" —
// real files with drift detection.)
export default module("hello", {
  config: tree("configs/hello", "~/.config/hello", "owned"),
});
