import {
  defineWorkspace, file, identity, literalText, managedBlock, profile, repoFile,
  repoTree, templateFile, trackedCopyTo, workspace,
} from "@gripsack/core";
import type { WorkspaceFile } from "@gripsack/core";

// Ordinary typed composition: no filesystem reads, generator or registry.
function editorFiles(theme: string): WorkspaceFile[] {
  return [
    file({
      source: repoFile("dotfiles/editor.toml.tmpl"),
      content: templateFile({ theme }),
      destination: trackedCopyTo("~/.config/editor/config.toml"),
    }),
    file({
      source: repoTree("dotfiles/editor", {
        include: ["theme.toml", "recipes"],
        exclude: ["recipes/drafts"],
      }),
      content: identity(),
      // Copies retain physical siblings and any captured aliases' logical paths.
      destination: trackedCopyTo("~/.config/editor/assets"),
    }),
  ];
}

const dotfiles = profile("dotfiles", {
  files: [
    ...editorFiles("dark"),
    file({
      source: repoFile("dotfiles/editor-theme.sh.tmpl"),
      content: templateFile({ theme: "dark" }),
      destination: trackedCopyTo("~/.local/bin/editor-theme"),
    }),
    file({
      content: literalText("export EDITOR=editor\n"),
      destination: managedBlock("~/.profile", "gripsack-editor"),
    }),
  ],
});

// Native files: check/plan prepare without deployment; apply uses generations.
export default defineWorkspace(() => workspace({ outputs: [dotfiles] }));
