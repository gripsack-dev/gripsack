import {
  defineWorkspace, environment, inputFile, pixi, pkg, provider,
  targetPlatform, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64" });
const inputs = [
  inputFile("pixi-manifest", "pixi.toml"),
  inputFile("pixi-lock", "pixi.lock"),
];
const tools = pkg("tools", {
  producer: provider(pixi.fromLock({
    manifest: "pixi-manifest",
    lock: "pixi-lock",
    environment: "default",
  })),
  commands: { rg: "bin/rg" },
  target,
  layout: { kind: "prefix_materialized" },
});
const search = environment("search", { packages: ["tools"], target });

export default defineWorkspace(() => workspace({ inputs, outputs: [tools, search] }));
