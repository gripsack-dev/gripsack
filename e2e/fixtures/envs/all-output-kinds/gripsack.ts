import {
  artifact, artifactFile, bash, bashBody, check, daily, defineWorkspace,
  environment, exec, file, fileFetch, hook, identity, image, lit,
  managedBlock, packageCommand, pkg, profile, provider, recipe,
  repoFile, schedule, symlinkTo, targetPlatform, task, templateText,
  trackedCopyTo, workspace,
} from "@gripsack/core";

const linux = targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" });
const host = { kind: "host", access: "unconfined" } as const;

const shell = pkg("shell", {
  producer: provider(fileFetch("shell.bin")),
  commands: { bash: "bin/bash" }, target: linux,
  layout: { kind: "relocatable" },
});
const fixed = pkg("fixed", {
  producer: provider(fileFetch("fixed.bin")),
  commands: { fixed: "bin/fixed" }, target: linux,
  layout: { kind: "fixed_prefix", prefix: "/opt/tools" },
});
const build = recipe("build", {
  source: fileFetch("recipe-source.txt"), execution: host,
  output_kind: "tree", target: linux,
  steps: [
    bash(packageCommand("shell", "bash"))
      .body(bashBody`
        echo "$INPUT"
      `)
      .env("INPUT", lit("from a typed environment value")).build(),
    exec(packageCommand("shell", "bash")).arg(lit("--version")).build(),
  ],
  checks: ["build-ok"],
});
const app = pkg("app", {
  producer: "build", commands: { app: "bin/app" },
  runtime: ["shell"], target: linux, layout: { kind: "relocatable" },
});
const buildOk = check("build-ok", {
  subject: "app",
  run: exec({ argv: [packageCommand("app", "app"), lit("--version")] }),
});
const dev = environment("dev", {
  packages: ["shell", "app", "fixed"], target: linux,
  prefix: "/opt/tools", env: { APP_ROOT: artifact("app", ".") },
});
const prepare = task("prepare", { run: exec({ argv: [lit("true")] }) });
const format = task("format", {
  run: exec(packageCommand("app", "app")).arg(lit("format")).build(),
  deps: ["prepare"], environment: "dev", checks: ["build-ok"],
});
const nightly = schedule("nightly", { task: "format", trigger: daily("03:30") });
const reload = hook("reload", {
  trigger: "post_activate", run: exec({ argv: [lit("true")] }),
});
const container = image("container", { packages: ["shell", "app"], target: linux });
const personal = profile("personal", {
  files: [
    file({
      source: repoFile("dotfiles/left/config.tmpl"),
      content: templateText("name={{ name }}\n", { name: "left" }),
      destination: symlinkTo("~/.config/demo/left"),
    }),
    file({
      source: repoFile("dotfiles/right/config.tmpl"),
      content: templateText("name={{ name }}\n", { name: "right" }),
      destination: trackedCopyTo("~/.config/demo/right"),
    }),
    file({
      source: artifactFile("app", "share/config.tmpl"),
      content: templateText("name={{ name }}\n", { name: "block" }),
      destination: managedBlock("~/.config/demo/shared", "#"),
    }),
    file({
      source: artifactFile("app", "share/readme"), content: identity(),
      destination: trackedCopyTo("~/.config/demo/readme"),
    }),
  ],
  environment: "dev", schedules: ["nightly"], hooks: ["reload"],
});

// Admission-only corpus: A2/E/B own every realization and registration.
export default defineWorkspace(() => workspace({
  outputs: [shell, fixed, build, app, buildOk, dev, prepare, format,
    nightly, reload, container, personal],
}));
