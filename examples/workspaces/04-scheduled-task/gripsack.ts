import {
  daily, defineWorkspace, environment, exec, fileFetch, lit, lock,
  packageCommand, pkg, profile, provider, schedule, targetPlatform,
  task, workspace,
} from "@gripsack/core";

const linux = targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" });
const formatter = pkg("formatter", {
  producer: provider(fileFetch("downloads/formatter.tar.gz")),
  commands: { format: "bin/formatter" },
  target: linux, layout: { kind: "relocatable" },
});
const dev = environment("dev", { packages: ["formatter"], target: linux });
const documents = lock("store", "format-documents");
const format = task("format", {
  steps: [exec(packageCommand("formatter", "format")).arg(lit("docs")).build()],
  environment: "dev",
  locks: [documents],
});
const timer = schedule("nightly", { task: "format", trigger: daily("03:30") });
const personal = profile("personal", { schedules: ["nightly"] });

// `grip task run format` and the timer invoke this same finite task.
// Only applying personal installs it; imports and previews activate nothing.
export default defineWorkspace(() => workspace({
  outputs: [formatter, dev, format, timer, personal],
}));
