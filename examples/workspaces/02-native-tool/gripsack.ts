import {
  defineWorkspace, environment, file, fileFetch, literalText, pkg,
  profile, provider, targetPlatform, trackedCopyTo, workspace,
} from "@gripsack/core";

const linux = targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" });
const formatter = pkg("formatter", {
  producer: provider(fileFetch("downloads/formatter.tar.gz")),
  commands: { format: "bin/formatter" },
  target: linux,
  layout: { kind: "relocatable" },
});
const development = environment("development", {
  packages: ["formatter"], target: linux,
});
const personal = profile("personal", {
  environment: "development",
  files: [file({
    content: literalText('indent = 2\n'),
    destination: trackedCopyTo("~/.config/formatter/config.toml"),
  })],
});

// The local archive stands in for previously downloaded bytes. A2
// owns acquisition, package realization and profile deployment.
export default defineWorkspace(() => workspace({
  outputs: [formatter, development, personal],
}));
