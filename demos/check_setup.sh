#!/bin/sh
# A genuine unknown workspace reference, diagnosed before deployment.
set -e
rm -rf /tmp/checkenv ~/.local/share/gripsack
mkdir -p /tmp/checkenv
cat > /tmp/checkenv/gripsack.ts <<'EOF'
import { defineWorkspace, environment, profile, targetPlatform, workspace } from "@gripsack/core";
const dev = environment("dev", {
  target: targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" }),
  packages: [],
});
const personal = profile("personal", { environment: "TYPO" });
export default defineWorkspace(() => workspace({ outputs: [dev, personal] }));
EOF
