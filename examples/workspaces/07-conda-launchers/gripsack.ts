import {
  conda, defineWorkspace, environment, pkg, profile, provider,
  targetPlatform, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64" });
const tools = pkg("tools", {
  producer: provider(conda.environment({
    channels: ["conda-forge"],
    packages: { ripgrep: "*" },
    systemRequirements: {
      libc: { family: "glibc", version: "2.28" },
      linux: "4.18",
    },
  })),
  commands: { rg: "bin/rg" },
  target,
  layout: { kind: "prefix_materialized" },
});
const search = environment("search", { packages: ["tools"], target });
const personal = profile("personal", {
  environment: "search",
  launchers: { directory: "~/.local/bin", commands: ["rg"] },
});

export default defineWorkspace(() => workspace({ outputs: [tools, search, personal] }));
