import {
  bash, conda, daily, defineWorkspace, environment, lit, packageCommand,
  pkg, profile, provider, schedule, targetPlatform, task, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64" });
const shell = pkg("shell", {
  producer: provider(conda.environment({
    channels: ["conda-forge"],
    packages: { bash: "*" },
    systemRequirements: {
      libc: { family: "glibc", version: "2.28" },
      linux: "4.18",
    },
  })),
  commands: { bash: "bin/bash" },
  target,
  layout: { kind: "prefix_materialized" },
});
const dev = environment("dev", { packages: ["shell"], target });
const greet = task("greet", {
  environment: "dev",
  steps: [
    bash`printf '%s\n' "$MESSAGE"`
      .interpreter(packageCommand("shell", "bash"))
      .env("MESSAGE", lit("two words"))
      .build(),
  ],
});
const nightly = schedule("nightly", { task: "greet", trigger: daily("03:30") });
const personal = profile("personal", { schedules: ["nightly"] });

export default defineWorkspace(() => workspace({
  outputs: [shell, dev, greet, nightly, personal],
}));
