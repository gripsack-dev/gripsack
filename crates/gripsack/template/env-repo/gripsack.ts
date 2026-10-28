import { defineWorkspace, workspace, profile, file, repoFile, identity, symlinkTo } from "@gripsack/core";

export default defineWorkspace(() => workspace({
  outputs: [
    profile("hello", {
      files: [
        file({
          source: repoFile("configs/hello/hello.toml"),
          content: identity(),
          destination: symlinkTo("~/.config/hello/hello.toml"),
        }),
      ],
    }),
  ],
}));
