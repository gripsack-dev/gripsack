#!/bin/sh
# Fixture env repo with a lint error for the check tape (rendered inside
# the VHS container; see .agents/skills/gripsack-demo-capture).
set -e

# idempotent renders: one container runs every tape × palette
rm -rf /tmp/checkenv ~/.local/share/gripsack

mkdir -p /tmp/checkenv/modules /tmp/checkenv/hosts /tmp/checkenv/configs/demo

cat > /tmp/checkenv/env.toml <<'EOF'
[env]
name = "demo"

[linters.demo]
path = "/vhs/demos/fixtures/griplint-demo"
EOF

cat > /tmp/checkenv/hosts/demo.py <<'EOF'
tags = ["demo"]
EOF

cat > /tmp/checkenv/modules/demo.py <<'EOF'
from gripsack import module, tracked_copy

module(
    "demo",
    config={"configs/demo/demo.toml": tracked_copy("~/.config/demo/demo.toml")},
    lint="demo",
)
EOF

cat > /tmp/checkenv/configs/demo/demo.toml <<'EOF'
theme = "catppuccin-mocha"
BAD_KEY = 1
EOF
