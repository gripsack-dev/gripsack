import {
  defineWorkspace, workspace, profile, file, literalText, trackedCopyTo,
  pkg, provider, fileFetch, targetPlatform,
} from "@gripsack/core";

export default defineWorkspace(() => workspace({
  outputs: [
    profile("dotfiles", {
      files: [file({
        content: literalText("setting=1\n"),
        destination: trackedCopyTo("~/.config/demo/settings.conf"),
      })],
    }),
    pkg("native-tool", {
      producer: provider(fileFetch("tool.bin")),
      commands: { tool: "bin/tool" },
      target: targetPlatform({ os: "linux", arch: "x86_64" }),
      layout: "relocatable",
    }),
  ],
}));
