import {
  defineWorkspace, environment, exec, fileFetch, lit, outputPath, packageCommand,
  pkg, provider, recipe, sourcePath, targetPlatform, task, workspace,
} from "@gripsack/core";

const linux = targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" });
const build = recipe("build-greeter", {
  source: fileFetch("source"),
  execution: {
    kind: "isolated_linux", worker: "buildkit", platform: linux,
    toolchain: {
      reference: "docker.io/library/golang@sha256:e3665e241a474aba30bbfaf177cfa88e1913e970c83bd86889cacfb67d6e7e51",
    },
  },
  output_kind: "tree", target: linux,
  steps: [
    exec({ argv: [lit("mkdir"), lit("-p"), outputPath("bin")] }),
    exec({ argv: [lit("cc"), lit("-static"), sourcePath("greeter.c"), lit("-o"), outputPath("bin/greeter")] }),
  ],
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
  const smoke = task("smoke", { steps: [exec({ argv: [packageCommand("greeter", "greet"), lit("hello")] })], environment: "dev", });
  // Build, project execution and this task do not activate a personal generation.
  return workspace({ outputs: [build, greeter, dev, smoke] });
});
