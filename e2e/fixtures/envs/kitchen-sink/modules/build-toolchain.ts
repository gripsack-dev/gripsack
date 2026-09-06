import { module, shellStep } from "@gripsack/core";

export default module("build-toolchain", {
  steps: [shellStep("mkdir -p bin && printf '#!/bin/sh\\necho fixture\\n' > bin/cc && chmod +x bin/cc", "build")],
});
