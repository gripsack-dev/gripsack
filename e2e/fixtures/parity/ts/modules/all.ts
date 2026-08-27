/** Parity fixture (typescript) — the python fixture's twin, field for
 *  field (plan/0007: the conformance test).
 */
import {
  brew, dep, desktopEntry, fileFetch, fonts, githubRelease, merge, module, pixi,
  pluginFetch, service, symlink, tarball, template, trackedCopy, tree, verifyBinary,
  verifyDeployed, verifyFile, verifyShell, when,
} from "@gripsack/core";
import { resource } from "@gripsack/core";

resource("parity.lock");

module("core", {
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
  depends: [dep("tools")],
  env: { PARITY_HOME: "{store}", "PATH+": "{store}/bin" },
  verify: verifyBinary("starship", ["--version"]),
  retries: 2,
  lint: "helix",
  activate: [fonts(), service("parity", true), desktopEntry()],
});

module("tools", {
  fetch: tarball("https://example.invalid/tools.tar.gz", "abababababababababababababababababababababababababababababababab"),
  config: { ...tree("configs/demo", "~/.config/demo", "owned") },
  verify: verifyShell("test -f bin/tools"),
  when: when({ tags: ["gui"] }),
});

module("extras", {
  fetch: fileFetch("payloads/x.tar.gz"),
  verify: verifyDeployed("~/.config/demo/a.toml"),
});

module("brewed", {
  fetch: brew("jq", "1.8.0"),
});

module("pixied", {
  fetch: pixi("ripgrep", "15.0.0"),
});

module("plugged", {
  fetch: pluginFetch("apt", { package: "htop", version: "3.3.0" }),
  verify: verifyFile("bin/htop"),
});
