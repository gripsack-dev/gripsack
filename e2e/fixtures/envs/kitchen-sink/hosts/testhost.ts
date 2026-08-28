// Kitchen-sink fixture host (plan/0013 D5) — gating lives HERE, not in
// module specs: fact conditions and probes are plain ctx reads, falsy
// module entries drop out. The golden corpus (e2e/test_golden.py)
// evaluates this env; tags and probe requests land in the snapshot.
//
// ctx.tags are the CLI --tags, NOT this host's own tags (they are the
// return value) — gating on host tags uses the local const.
import { defineEnv, hasTag } from "@gripsack/core";
import brewed from "../modules/brewed.ts";
import core from "../modules/core.ts";
import cuda from "../modules/cuda.ts";
import extras from "../modules/extras.ts";
import nvidia from "../modules/nvidia.ts";
import pixied from "../modules/pixied.ts";
import plugged from "../modules/plugged.ts";
import tools from "../modules/tools.ts";

const tags = ["gui"];

export default defineEnv((ctx) => ({
  tags,
  modules: [
    core,
    tags.includes("gui") && tools,
    hasTag("cli", ctx) && extras,
    brewed,
    pixied,
    plugged,
    ctx.facts.os === "linux" && nvidia,
    ctx.facts.libc?.startsWith("glibc") && ctx.probe.executable("nvidia-smi") && cuda,
  ],
}));
