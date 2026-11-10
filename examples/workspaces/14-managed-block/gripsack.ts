import {
  defineWorkspace, file, literalText, managedBlock, profile, workspace,
} from "@gripsack/core";

const dotfiles = profile("dotfiles", {
  files: [file({
    content: literalText("export EDITOR=vi\n"),
    destination: managedBlock("~/.profile", "gripsack-editor"),
  })],
});

export default defineWorkspace(() => workspace({ outputs: [dotfiles] }));
