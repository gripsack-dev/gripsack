import assert from "node:assert/strict";
import * as api from "../src/index.ts";
import { emitWorkspaceIr } from "../src/advanced.ts";
import type { ArchiveRootPolicy, ProfileLaunchers, RepoTreeSelection } from "../src/index.ts";

const target = { os: "linux", arch: "x86_64" } as const;
const layout = { kind: "relocatable" } as const;
const facts = { ...target, hostname: "test", libc: null };
const archiveRoot: ArchiveRootPolicy = { kind: "single_directory" };
const span = { file: "gripsack.ts", line: 12, col: 3 };
const tool = (name = "tools") => api.pkg(name, {
  producer: api.provider(api.tarball("https://example.invalid/tools.tar.gz")),
  commands: { tool: "bin/tool" }, target, layout, archiveRoot, span,
});
const emit = (...outputs: api.WorkspaceOutput[]) => JSON.parse(emitWorkspaceIr(api.workspace({ outputs }), facts, []));

Deno.test("repoTree is a pure captured declaration with normalized prefix selection", () => {
  const selection: RepoTreeSelection = { include: ["nested"], exclude: ["nested/cache"] };
  const source = api.repoTree("not-present-at-evaluation", selection);
  for (const path of ["", "/etc", "../x", "a/../b", "a//b", "a/", "a\\b", "*.toml", "a?", "[x]", "{x}", "a\nb"]) {
    assert.throws(() => api.repoTree(path));
  }
  for (const prefix of [".", "../x", "/x", "a//b", "a/", "*.toml", "a?", "[x]", "{x}", "a\nb"]) {
    assert.throws(() => api.repoTree(".", { include: [prefix] }));
    assert.throws(() => api.repoTree(".", { exclude: [prefix] }));
  }
  assert.throws(() => api.repoTree(".", { include: "abc" } as never));
  assert.throws(() => api.repoTree(".", { maxEntries: 1 } as never));
  for (const content of [api.literalText("x"), api.templateText("{{x}}", { x: "y" }), api.templateFile({ x: "y" })]) {
    assert.throws(() => api.file({ source, content, destination: api.symlinkTo("~/.config/tree") }));
    assert.throws(() => api.file({ source: api.artifactTree("tools", { include: ["share"] }), content, destination: api.symlinkTo("~/.config/tree") }));
  }
});

Deno.test("captured templates reject invalid variables, digest claims and absent sources", () => {
  const content = api.templateFile({ editor: "hx" }, "a".repeat(64));
  assert.throws(() => api.templateFile({ x: 1 } as never));
  assert.throws(() => api.templateFile({}, "bad"));
  assert.throws(() => api.file({ content, destination: api.trackedCopyTo("~/.config/tool") }));
});

Deno.test("archive root selection rejects incompatible producer and layout policies", () => {
  const base = { producer: api.provider(api.tarball("https://example.invalid/tool.tar.gz")), commands: { tool: "bin/tool" }, target, layout };
  assert.throws(() => api.pkg("bad", { ...base, archiveRoot: { kind: "other" } as never }));
  assert.throws(() => api.pkg("bad", { ...base, archiveRoot, layout: { kind: "fixed_prefix", prefix: "/opt/tools" } }));
  const condaSource = api.condaEnvironment({ channels: ["conda-forge"], packages: { python: "*" }, platforms: [target] });
  assert.throws(() => api.pkg("bad", { ...base, producer: api.provider(condaSource), archiveRoot }));
  const recipe = api.recipe("source", { source: condaSource, execution: { kind: "host", access: "unconfined" }, output_kind: "tree", target });
  assert.throws(() => emit(recipe, api.pkg("bad", { ...base, producer: "source", archiveRoot })));
});

Deno.test("managed launchers require an explicit unique command selection and environment", () => {
  const launchers: ProfileLaunchers = { directory: "~/.local/bin", commands: ["tool"] };
  const profile = api.profile("desktop", { environment: "editor", launchers, span });
  const environment = api.environment("editor", { packages: ["tools"], target });
  assert.throws(() => api.profile("bad", { launchers }));
  for (const directory of ["", "relative", "~/../bin", "/a//b"]) {
    assert.throws(() => api.profile("bad", { environment: "editor", launchers: { ...launchers, directory } }));
  }
  for (const commands of [[], ["tool", "tool"], ["../tool"], ["."], ["a/b"], ["a b"]]) {
    assert.throws(() => api.profile("bad", { environment: "editor", launchers: { ...launchers, commands } }));
  }
  assert.throws(() => emit(tool(), environment, api.profile("bad", { environment: "editor", launchers: { ...launchers, commands: ["missing"] } })));
  assert.throws(() => emit(tool(), tool("other"), api.environment("editor", { packages: ["tools", "other"], target }), profile));
});
