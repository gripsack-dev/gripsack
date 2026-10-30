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

Deno.test("Conda wheels emit exact binary requests and inherit source provenance", () => {
  const wheel = { name: "tool", version: "1.0", url: "https://example.invalid/tool-1.0-py3-none-any.whl", sha256: "a".repeat(64) };
  const wheels: readonly api.WheelSource[] = [wheel];
  const source = api.conda.environment({
    channels: ["conda-forge"], packages: { python: "3.12.*" }, wheels,
  });
  const output = api.pkg("python", {
    producer: api.provider(source), commands: { python: "bin/python" },
    target, layout: { kind: "prefix_materialized" },
  });
  const wire = emit(output).workspace.outputs[0].producer.provider;
  assert.deepEqual(wire.wheels, [wheel]);
  assert.deepEqual(wire.span, source.span);
  assert.equal("span" in wire.wheels[0], false);
  wheel.sha256 = "b".repeat(64);
  assert.equal(source.wheels![0]!.sha256, "a".repeat(64));
  assert.ok(Object.isFrozen(source.wheels));
  assert.ok(Object.isFrozen(source.wheels![0]));
  const plain = api.conda.environment({ channels: ["conda-forge"], packages: { python: "*" } });
  assert.equal("wheels" in plain, false);
});

Deno.test("Conda wheel requests reject unpinned, source and editable requests", () => {
  const wheel = { name: "tool", version: "1.0", url: "https://example.invalid/tool-1.0-py3-none-any.whl", sha256: "a".repeat(64) };
  for (const invalid of [
    null, {}, { url: wheel.url }, { ...wheel, sha256: "A".repeat(64) },
    { ...wheel, sha256: "a".repeat(63) }, { ...wheel, url: "http://example.invalid/tool.whl" },
    { ...wheel, url: "file:///tmp/tool.whl" }, { ...wheel, url: "https://example.invalid/tool.tar.gz" },
    { ...wheel, url: "https://user:secret@example.invalid/tool.whl" },
    { ...wheel, url: "https://example.invalid/tool.whl#hash" },
    { ...wheel, url: "https://example.invalid/tool.whl?token=secret" },
    { ...wheel, url: "https://example.invalid/tool%2D1.whl" },
    { ...wheel, url: "https://example.invalid/tool\n.whl" },
    { ...wheel, name: "Tool" }, { ...wheel, name: "my_tool" }, { ...wheel, name: "my.tool" },
    { ...wheel, version: "" }, { ...wheel, version: " 1.0" }, { ...wheel, version: "1.0\n" },
    { ...wheel, version: "1/0" }, { ...wheel, version: "1".repeat(257) },
    { ...wheel, editable: true }, { ...wheel, source: true },
  ]) {
    assert.throws(() => api.conda.environment({
      channels: ["conda-forge"], packages: { python: "*" }, wheels: [invalid] as never,
    }));
    assert.throws(() => api.provider({
      kind: "conda_environment", channels: ["conda-forge"], packages: { python: "*" },
      wheels: [invalid] as never, span,
    }));
  }
  for (const wheels of [null, "tool", {}]) {
    assert.throws(() => api.conda.environment({
      channels: ["conda-forge"], packages: { python: "*" }, wheels: wheels as never,
    }));
  }
  assert.doesNotThrow(() => api.conda.environment({
    channels: ["conda-forge"], packages: { python: "*" },
    wheels: [{ ...wheel, version: "1!1.0+local" }],
  }));
  for (const duplicate of [
    { ...wheel, url: "https://example.invalid/other.whl", sha256: "b".repeat(64) },
    { ...wheel, name: "other", sha256: "b".repeat(64) },
    { ...wheel, name: "other", url: "https://example.invalid/other.whl" },
  ]) {
    assert.throws(() => api.conda.environment({
      channels: ["conda-forge"], packages: { python: "*" }, wheels: [wheel, duplicate],
    }), /duplicate wheel/);
  }
});

Deno.test("v10 removes executable authoring exports and recipe producers", () => {
  for (const retired of ["recipe", "image", "cargoPackage", "bashBody"]) {
    assert.equal(retired in api, false);
  }
  for (const producer of ["old-recipe", { kind: "recipe", recipe: "old-recipe" }]) {
    assert.throws(() => api.pkg("invalid", {
      producer: producer as never, commands: {}, target, layout,
    }));
  }
});

Deno.test("portable package sources emit only an exact manifest digest and provenance", () => {
  const manifest = "a".repeat(64);
  const source = api.source.package({ manifest });
  const packageValue = api.pkg("portable", {
    producer: api.provider(source), commands: { tool: "bin/tool" }, target, layout,
  });
  const wire = emit(packageValue).workspace.outputs[0].producer.provider;
  assert.deepEqual(wire, { kind: "package", manifest, span: source.span });
  assert.ok(Object.isFrozen(source));
  for (const invalid of ["", "a".repeat(63), "A".repeat(64), "g".repeat(64), `sha256:${manifest}`]) {
    assert.throws(() => api.source.package({ manifest: invalid }), /64-hex SHA256/);
  }
  assert.throws(() => api.source.package({ manifest, transport: "registry" } as never));
});

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
  const condaSource = api.conda.environment({ channels: ["conda-forge"], packages: { python: "*" }, platforms: [target] });
  assert.throws(() => api.pkg("bad", { ...base, producer: api.provider(condaSource), archiveRoot }));
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

