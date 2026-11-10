import {
  check, defineWorkspace, exec, file, identity, lit, profile, repoFile,
  sourcePath, trackedCopyTo, workspace,
} from "@gripsack/core";

const syntax = check("shell-syntax", {
  subject: "dotfiles",
  run: exec({ argv: [lit("/bin/sh"), lit("-n"), sourcePath(".")] }),
});
const dotfiles = profile("dotfiles", {
  files: [file({
    source: repoFile("config/shell.sh"),
    content: identity(),
    destination: trackedCopyTo("~/.config/shell/settings.sh"),
    checks: [{ check: "shell-syntax", subject: "rendered", stage: "pre_flip" }],
  })],
});

export default defineWorkspace(() => workspace({ outputs: [syntax, dotfiles] }));
