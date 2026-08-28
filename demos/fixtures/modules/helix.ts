import { dep, githubRelease, module } from "@gripsack/core";

export default module("helix", {
  fetch: githubRelease({ repo: "helix-editor/helix", asset: "h.tar.xz" }),
  depends: [dep("nvim")],
});
