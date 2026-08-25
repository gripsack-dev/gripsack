"""The gripsack feature tour — every block below is commented out so
`grip check` passes offline. Uncomment one at a time as you need it.

Docs: https://gripsack.dev/docs/modules.html
"""

# ── Fetch a binary: pinned, verified, rolled back like everything else ──
#
# from gripsack import module, github_release, symlink, verify_binary
#
# module(
#     "ripgrep",
#     # First apply resolves the release and writes grip.lock — commit it.
#     # `grip update ripgrep` moves the pin deliberately.
#     fetch=github_release("BurntSushi/ripgrep", "{version}-x86_64-unknown-linux-musl.tar.gz"),
#     install={"bin/rg": symlink("~/.local/bin/rg")},
#     verify=verify_binary("~/.local/bin/rg", args=["--version"]),
# )

# ── Ownership modes (0001 §3.7) ──
#
# from gripsack import module, symlink, tracked_copy, merge, template
#
# module(
#     "zed",
#     config={
#         # owned: read-only symlink into the store — disciplined tools.
#         "configs/zed/keymap.json": symlink("~/.config/zed/keymap.json"),
#         # tracked_copy: a real file; drift is detected, never silently
#         # overwritten. For apps that rewrite their own configs.
#         "configs/zed/settings.json": tracked_copy("~/.config/zed/settings.json"),
#     },
# )
#
# module(
#     "shell",
#     config={
#         # merge: gripsack owns ONE delimited block inside a file other
#         # tools also write. Everything outside the markers is never
#         # touched; prune removes only the block.
#         "configs/shell/path.sh": merge("~/.bashrc"),
#     },
# )

# ── Templates: per-host content from one file ──
#
# from gripsack import facts, module, template
#
# module(
#     "git",
#     config={
#         # configs/git/id.toml contains: email = "{{ email }}"
#         # Rendered at deploy time; undefined variables fail loudly.
#         "configs/git/id.toml": template(
#             "~/.config/git/id.toml",
#             vars={"email": "work@corp.example" if facts.has("work") else "me@home.example"},
#         ),
#     },
# )

# ── Module dependencies and multi-step builds ──
#
# from gripsack import dep, module, run_step
#
# module("font", fetch=..., install=...)
# module(
#     "terminal",
#     depends=[dep("font")],            # built before this module
#     steps=[run_step("fc-cache -f")],  # then shell out (last rung)
# )
