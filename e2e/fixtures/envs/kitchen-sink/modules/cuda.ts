// Probe-gated module (plan/0013 D6): ctx.probe.executable records a
// symbolic request in round 1 and returns the core-bound answer in
// round 2 — the golden corpus snapshots the round-1 request.
import { fileFetch, module, symlink } from "@gripsack/core";

export default module("cuda", {
  fetch: fileFetch("payloads/cuda.tar.gz"),
  install: { "bin/nvcc": symlink("~/.local/bin/nvcc") },
});
