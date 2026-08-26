#!/bin/sh
# Fixture env repo with a lint error for the check tape (rendered inside
# the VHS container; see .agents/skills/gripsack-demo-capture).
set -e

mkdir -p /tmp/myenv/modules /tmp/myenv/hosts /tmp/myenv/configs/demo

cat > /tmp/myenv/env.toml <<'EOF'
[env]
name = "demo"

[linters.demo]
path = "/vhs/demos/fixtures/griplint-demo"
EOF

cat > /tmp/myenv/hosts/demo.py <<'EOF'
tags = ["demo"]
EOF

cat > /tmp/myenv/modules/demo.py <<'EOF'
from gripsack import module, tracked_copy

module(
    "demo",
    config={"configs/demo/demo.toml": tracked_copy("~/.config/demo/demo.toml")},
)
EOF

cat > /tmp/myenv/configs/demo/demo.toml <<'EOF'
theme = "catppuccin-mocha"
BAD_KEY = 1
EOF
