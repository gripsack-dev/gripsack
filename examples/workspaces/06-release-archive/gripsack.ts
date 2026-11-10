import {
  defineWorkspace, environment, githubRelease, pkg, provider,
  targetPlatform, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64" });
const ripgrep = pkg("ripgrep", {
  producer: provider(githubRelease({
    repo: "BurntSushi/ripgrep",
    asset: "ripgrep-{version.bare}-x86_64-unknown-linux-musl.tar.gz",
  })),
  archiveRoot: { kind: "single_directory" },
  commands: { rg: "rg" },
  target,
  layout: { kind: "relocatable" },
});
const search = environment("search", { packages: ["ripgrep"], target });

export default defineWorkspace(() => workspace({ outputs: [ripgrep, search] }));
