import { dep, installStep, module, shellStep, symlink } from "@gripsack/core";

export default module("build-consumer", {
  depends: [dep("build-toolchain", { for: "build" })],
  steps: [
    shellStep("mkdir -p out && cp \"$(command -v cc)\" out/built", "build"),
    installStep({ "out/built": symlink("~/.local/bin/fixture-built") }, "install", { needs: ["build"] }),
  ],
});
