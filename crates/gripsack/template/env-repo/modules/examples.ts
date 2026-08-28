// The gripsack feature tour — every block below is commented out so
// `grip check` passes offline. Uncomment one at a time as you need it.
//
// A module is a pure value: it registers nothing until your host
// entrypoint (hosts/<hostname>.ts) returns it in `modules`.
//
// Docs: https://gripsack.dev/docs/modules.html

// ── Fetch a binary: pinned, verified, rolled back like everything else ──
//
// import { githubRelease, module, symlink, verifyBinary } from "@gripsack/core";
//
// export default module("ripgrep", {
//   // First apply resolves the release and writes grip.lock — commit it.
//   // `grip update ripgrep` moves the pin deliberately.
//   fetch: githubRelease({
//     repo: "BurntSushi/ripgrep",
//     asset: "ripgrep-{version}-x86_64-unknown-linux-musl.tar.gz",
//   }),
//   install: { "bin/rg": symlink("~/.local/bin/rg") },
//   verify: verifyBinary("~/.local/bin/rg", ["--version"]),
// });

// ── Ownership modes (0001 §3.7) ──
//
// import { merge, module, symlink, trackedCopy } from "@gripsack/core";
//
// export default module("zed", {
//   config: {
//     // owned: read-only symlink into the store — disciplined tools.
//     "configs/zed/keymap.json": symlink("~/.config/zed/keymap.json"),
//     // tracked_copy: a real file; drift is detected, never silently
//     // overwritten. For apps that rewrite their own configs.
//     "configs/zed/settings.json": trackedCopy("~/.config/zed/settings.json"),
//   },
// });
//
// export default module("shell", {
//   config: {
//     // merge: gripsack owns ONE delimited block inside a file other
//     // tools also write. Everything outside the markers is never
//     // touched; prune removes only the block.
//     "configs/shell/path.sh": merge("~/.bashrc", "#"),
//   },
// });

// ── Templates: per-host content from one file ──
//
// import { module, template } from "@gripsack/core";
//
// export default module("git", {
//   config: {
//     // configs/git/id.toml contains: email = "{{ email }}"
//     // Rendered at deploy time; undefined variables fail loudly.
//     // Per-host variation lives in the host entrypoint: gate the whole
//     // module on ctx.facts / ctx.tags there, or export two variants.
//     "configs/git/id.toml": template("~/.config/git/id.toml", {
//       email: "me@home.example",
//     }),
//   },
// });

// ── Module dependencies and multi-step builds ──
//
// import { dep, module, runStep } from "@gripsack/core";
//
// export const font = module("font", { fetch: ..., install: ... });
// export default module("terminal", {
//   depends: [dep("font")],        // built before this module
//   steps: [runStep("fc-cache", ["-f"])],  // structured argv, no shell
// });
