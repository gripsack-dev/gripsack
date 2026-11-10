import {
  defineWorkspace, file, identity, profile, repoFile, repoTree,
  templateFile, trackedCopyTo, workspace,
} from "@gripsack/core";

const dotfiles = profile("dotfiles", {
  files: [
    file({
      source: repoTree("dotfiles/editor"),
      content: identity(),
      destination: trackedCopyTo("~/.config/editor"),
    }),
    file({
      source: repoFile("scripts/editor-theme.sh.tmpl"),
      content: templateFile({ theme: "dark" }),
      destination: trackedCopyTo("~/.local/bin/editor-theme"),
    }),
  ],
});

export default defineWorkspace(() => workspace({ outputs: [dotfiles] }));
