// Every declarative IR field shape on one module — the golden corpus's
// widest single row (IR-only fixture; URLs never fetch).
import {
  dep,
  desktopEntry,
  fonts,
  githubRelease,
  merge,
  module,
  service,
  symlink,
  template,
  trackedCopy,
  verifyBinary,
} from "@gripsack/core";

export default module("core", {
  fetch: githubRelease({
    repo: "starship/starship",
    asset: "starship-{version}-x86_64-linux.tar.gz",
    version: "v1.20.0",
    base_url: "https://ghe.example.com/api/v3",
  }),
  install: { starship: symlink("~/.local/bin/starship") },
  config: {
    "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml"),
    "configs/demo/id.toml": template("~/.config/demo/id.toml", { email: "a@b.c" }),
    "configs/demo/block.sh": merge("~/.bashrc", "#"),
  },
  env: { SINK_HOME: "{store}", "PATH+": "{store}/bin" },
  verify: verifyBinary("starship", ["--version"]),
  retries: 2,
  lint: "helix",
  activate: [fonts(), service("kitchen-sink", true), desktopEntry()],
});
