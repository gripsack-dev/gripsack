#!/bin/sh
# Fixture env repo for the demo tape (rendered inside the VHS container;
# see .agents/skills/gripsack-demo-capture).
set -e

# idempotent renders: one container runs every tape × palette
rm -rf /tmp/myenv ~/.local/share/gripsack

mkdir -p /tmp/myenv/modules /tmp/myenv/hosts

cat > /tmp/myenv/env.toml <<'EOF'
[env]
name = "demo"
EOF

cat > /tmp/myenv/hosts/demo.ts <<'EOF'
import { defineEnv } from "@gripsack/core";
import hello from "../modules/hello.ts";
import editor from "../modules/editor.ts";

export default defineEnv(() => ({
  tags: ["demo"],
  modules: [hello, editor],
}));
EOF

mkdir -p /tmp/payload/bin
printf '#!/bin/sh\necho "hello from gripsack"\n' > /tmp/payload/bin/hello
chmod +x /tmp/payload/bin/hello
tar -czf /tmp/myenv/hello.tar.gz -C /tmp/payload bin

cat > /tmp/myenv/modules/hello.ts <<'EOF'
import { fileFetch, module, symlink, verifyBinary } from "@gripsack/core";

export default module("hello", {
  fetch: fileFetch("/tmp/myenv/hello.tar.gz"),
  install: { "bin/hello": symlink("~/.local/bin/hello") },
  verify: verifyBinary("bin/hello"),
});
EOF

cat > /tmp/myenv/modules/editor.ts <<'EOF'
import { module, trackedCopy } from "@gripsack/core";

export default module("editor", {
  config: { "editor.toml": trackedCopy("~/.config/editor/editor.toml") },
});
EOF

cat > /tmp/myenv/editor.toml <<'EOF'
theme = "catppuccin-mocha"
EOF
