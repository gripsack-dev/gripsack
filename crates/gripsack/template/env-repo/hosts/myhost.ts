// Host entrypoint for this machine — selected by `grip apply --host <name>`
// (default: the machine's hostname, so name the file after it).
//
// A host is a function (plan/0013): gripsack calls it with `ctx` — the
// machine's facts (os, arch, libc, hostname), your CLI tags, declared
// probes — and you return the environment. Gating is plain code, and
// falsy module entries drop out:
//
//     modules: [
//       hello,
//       ctx.facts.os === "linux" && linuxTools,
//       ctx.probe.executable("nvidia-smi") && cuda,
//     ],
//
// Evaluation is sandboxed — no env vars, no network, no subprocesses.
// Everything the config may know about this machine arrives via `ctx`.

import { defineEnv } from "@gripsack/core";
import hello from "../modules/hello.ts";

export default defineEnv(() => ({
  tags: [
    // "work",
    // "laptop",
  ],
  modules: [hello],
}));
