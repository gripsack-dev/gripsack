// Fact-gated module (plan/0013 D4/D5): the host reads ctx.facts —
// injected by the core, never detected here.
import { fileFetch, module, symlink } from "@gripsack/core";

export default module("nvidia", {
  fetch: fileFetch("payloads/nvidia.tar.gz"),
  install: { "bin/nvidia-smi": symlink("~/.local/bin/nvidia-smi") },
});
