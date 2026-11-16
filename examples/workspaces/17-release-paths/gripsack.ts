import {
  artifactFile, defineWorkspace, environment, file, githubRelease, identity,
  pkg, profile, provider, targetPlatform, trackedCopyTo, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64" });
const payload = "sops-{version}.{os}.{arch.go}";
const sops = pkg("sops", {
  producer: provider(githubRelease({
    repo: "getsops/sops",
    asset: payload,
  })),
  commands: { sops: payload },
  target,
  layout: { kind: "relocatable" },
});
const secrets = environment("secrets", { packages: ["sops"], target });
const tools = profile("tools", {
  files: [
    file({
      source: artifactFile("sops", payload),
      content: identity(),
      destination: trackedCopyTo("~/.local/bin/sops"),
    }),
  ],
});

export default defineWorkspace(() => workspace({ outputs: [sops, secrets, tools] }));
