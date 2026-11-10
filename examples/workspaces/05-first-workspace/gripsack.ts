import {
  defineWorkspace, file, identity, profile, repoFile, trackedCopyTo, workspace,
} from "@gripsack/core";

const dotfiles = profile("dotfiles", {
  files: [file({
    source: repoFile("config/editor.toml"),
    content: identity(),
    destination: trackedCopyTo("~/.config/editor/config.toml"),
  })],
});

export default defineWorkspace(() => workspace({ outputs: [dotfiles] }));
