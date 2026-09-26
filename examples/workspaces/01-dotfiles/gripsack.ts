import {
  defineWorkspace, file, managedBlock, profile, repoFile, templateText,
  trackedCopyTo, workspace, literalText,
} from "@gripsack/core";

const dotfiles = profile("dotfiles", {
  files: [
    file({
      source: repoFile("dotfiles/editor.toml.tmpl"),
      content: templateText('theme = "{{ theme }}"\n', { theme: "dark" }),
      destination: trackedCopyTo("~/.config/editor/config.toml"),
    }),
    file({
      content: literalText("export EDITOR=editor\n"),
      destination: managedBlock("~/.profile", "gripsack-editor"),
    }),
  ],
});

// A1 admits the file contract; A2 owns rendering and deployment.
export default defineWorkspace(() => workspace({ outputs: [dotfiles] }));
