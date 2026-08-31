#!/bin/sh
# Pre-existing helix config for the adopt tape — the whole point is that
# these files exist BEFORE gripsack (rendered inside the VHS container;
# see .agents/skills/gripsack-demo-capture).
set -e

# idempotent renders: one container runs every tape × palette
rm -rf /tmp/myenv ~/.local/share/gripsack ~/.config/helix

mkdir -p ~/.config/helix
cat > ~/.config/helix/config.toml <<'EOF'
theme = "catppuccin-mocha"
EOF
cat > ~/.config/helix/languages.toml <<'EOF'
# hand-tuned over the years
EOF
