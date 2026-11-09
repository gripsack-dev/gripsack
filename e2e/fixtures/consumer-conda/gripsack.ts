import {
  conda, defineWorkspace, environment, exec, lit, packageCommand, pkg,
  profile, provider, targetPlatform, task, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" });
const ripgrep = pkg("ripgrep", {
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

export default defineWorkspace(() => workspace({
  outputs: [
    ripgrep,
    environment("search", { packages: ["ripgrep"], target }),
    profile("personal", { environment: "search" }),
    task("version", {
      steps: [exec(packageCommand("ripgrep", "rg")).arg(lit("--version")).build()],
    }),
  ],
}));
