import {
  defineWorkspace, environment, exec, file, fileFetch, lit, literalText,
  pkg, profile, provider, targetPlatform, task, trackedCopyTo, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64" });
const tools = pkg("tools", {
  producer: provider(fileFetch("tool.bin")),
  commands: {},
  target, layout: { kind: "relocatable" },
});
const dev = environment("dev", {
  packages: ["tools"], target, env: { MODE: lit("offline") },
});
const invoke = task("invoke", {
  environment: "dev",
  steps: [exec(lit("/usr/bin/true"))
    .env("INPUT", lit("from a typed environment value")).build()],
});
const personal = profile("personal", {
  environment: "dev",
  files: [file({
    content: literalText("setting=1\n"),
    destination: trackedCopyTo("~/.config/demo/settings.conf"),
  })],
});

export default defineWorkspace(() => workspace({
  outputs: [tools, dev, invoke, personal],
}));
