import {
  daily, defineWorkspace, environment, exec, fileFetch, lit,
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
const manual = task("format-now", {
  run: exec(packageCommand("formatter", "format")).arg(lit("docs")).build(),
  environment: "dev",
});
const nightly = task("format-nightly", {
  run: exec(packageCommand("formatter", "format")).arg(lit("docs")).build(),
  environment: "dev", deps: ["format-now"],
});
const timer = schedule("nightly", { task: "format-nightly", trigger: daily("03:30") });
const personal = profile("personal", { schedules: ["nightly"] });

// A1 checks references and calendar syntax only; E1/E3 own execution
// and registration. Merely declaring this schedule activates nothing.
export default defineWorkspace(() => workspace({
  outputs: [formatter, dev, manual, nightly, timer, personal],
}));
