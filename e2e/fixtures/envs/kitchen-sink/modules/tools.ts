import { module, tarball, tree, verifyShell } from "@gripsack/core";

export default module("tools", {
  fetch: tarball(
    "https://example.invalid/tools.tar.gz",
    "abababababababababababababababababababababababababababababababab",
  ),
  config: { ...tree("configs/demo", "~/.config/demo", "owned") },
  verify: verifyShell("test -f bin/tools"),
});
