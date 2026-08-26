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

cat > /tmp/myenv/hosts/demo.py <<'EOF'
tags = ["demo"]
EOF

mkdir -p /tmp/payload/bin
printf '#!/bin/sh\necho "hello from gripsack"\n' > /tmp/payload/bin/hello
chmod +x /tmp/payload/bin/hello
tar -czf /tmp/myenv/hello.tar.gz -C /tmp/payload bin

cat > /tmp/myenv/modules/hello.py <<'EOF'
from gripsack import module, file_fetch, symlink, verify_binary

module(
    "hello",
    fetch=file_fetch("/tmp/myenv/hello.tar.gz"),
    install={"bin/hello": symlink("~/.local/bin/hello")},
    verify=verify_binary("bin/hello"),
)
EOF

cat > /tmp/myenv/modules/dotfiles.py <<'EOF'
from gripsack import module, tracked_copy

module(
    "editor",
    config={"editor.toml": tracked_copy("~/.config/editor/editor.toml")},
)
EOF

cat > /tmp/myenv/editor.toml <<'EOF'
theme = "catppuccin-mocha"
EOF
