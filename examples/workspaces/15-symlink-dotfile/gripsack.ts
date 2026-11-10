// gripsack.ts
import {
  defineWorkspace, workspace, profile, file, repoFile, identity, symlinkTo,
} from "@gripsack/core";

export default defineWorkspace(() => workspace({
  outputs: [profile("dotfiles", {
    files: [file({
      source: repoFile("config/editor.toml"),
      content: identity(),
      destination: symlinkTo("~/.config/editor/config.toml"),
    })],
  })],
}));
