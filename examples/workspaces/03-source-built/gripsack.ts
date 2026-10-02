import {
  defineWorkspace, environment, exec, fileFetch, lit, packageCommand,
  pkg, provider, recipe, targetPlatform, task, workspace,
} from "@gripsack/core";

const linux = targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" });
const build = recipe("build-greeter", {
  source: fileFetch("source/greeter.c"),
  execution: { kind: "host", access: "unconfined" },
  output_kind: "tree", target: linux,
  steps: [exec({ argv: [lit("sh"), lit("-c"), lit("mkdir -p bin && cc greeter.c -o bin/greeter")] })],
});

// The fact is injected, never read from the evaluator's ambient host.
// Both producers describe the same x86_64 package/command contract.
export default defineWorkspace((ctx) => {
  const greeter = pkg("greeter", {
    producer: ctx.facts.arch === "x86_64"
      ? "build-greeter"
      : provider(fileFetch("downloads/greeter.tar.gz")),
    commands: { greet: "bin/greeter" },
    target: linux, layout: { kind: "relocatable" },
  });
  const dev = environment("dev", { packages: ["greeter"], target: linux });
  const smoke = task("smoke", {
    run: exec({ argv: [packageCommand("greeter", "greet"), lit("hello")] }),
    environment: "dev",
  });
  // A1 admits source/consumer wiring; B3 and A2-P own realization/run.
  return workspace({ outputs: [build, greeter, dev, smoke] });
});
