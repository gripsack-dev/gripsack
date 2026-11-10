import {
  conda, defineWorkspace, environment, pkg, provider, targetPlatform, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64" });
const tools = pkg("tools", {
  producer: provider(conda.environment({
    channels: ["conda-forge"],
    packages: { python: "3.12.*", ripgrep: "*" },
    systemRequirements: {
      libc: { family: "glibc", version: "2.28" }, linux: "4.18",
    },
  })),
  commands: { python: "bin/python", rg: "bin/rg" },
  target,
  layout: { kind: "prefix_materialized" },
});

export default defineWorkspace(() => workspace({
  outputs: [tools, environment("dev", { packages: ["tools"], target })],
}));
