#!/bin/sh
# Disposable file-profile workspace for VHS; no recordings are generated here.
set -e
rm -rf /tmp/myenv ~/.local/share/gripsack
mkdir -p /tmp/myenv/scripts
cat > /tmp/myenv/gripsack.ts <<'EOF'
import {
  defineWorkspace, file, identity, profile, repoFile, trackedCopyTo, workspace,
} from "@gripsack/core";
const personal = profile("personal", {
  files: [
    file({ source: repoFile("scripts/hello"), content: identity(),
      destination: trackedCopyTo("~/.local/bin/hello") }),
    file({ source: repoFile("editor.toml"), content: identity(),
      destination: trackedCopyTo("~/.config/editor/editor.toml") }),
  ],
});
export default defineWorkspace(() => workspace({ outputs: [personal] }));
EOF
printf '#!/bin/sh\nprintf "hello from gripsack\\n"\n' > /tmp/myenv/scripts/hello
chmod +x /tmp/myenv/scripts/hello
printf 'theme = "catppuccin-mocha"\n' > /tmp/myenv/editor.toml
