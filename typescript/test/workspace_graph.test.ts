import assert from "node:assert/strict";
import { tarball } from "../src/fetch.ts";
import { emitWorkspaceIr } from "../src/advanced.ts";
import {
  artifact,
  artifactFile,
  artifactTree,
  check,
  conda,
  environment,
  exec,
  file,
  hook,
  identity,
  inputFile,
  lit,
  literalText,
  managedBlock,
  outputPath,
  packageCommand,
  pixi,
  pkg,
  profile,
  provider,
  repoTree,
  source,
  sourcePath,
  symlinkTo,
  task,
  trackedCopyTo,
  workspace,
} from "../src/workspace.ts";
import type { PackageLayout, WorkspaceDestination, WorkspaceOutput, WorkspacePlatform, WorkspaceValue } from "../src/workspace.ts";
import { thrownDiagnostic } from "./diagnostic.ts";

const facts = { os: "linux", arch: "x86_64", libc: null, hostname: "builder" } as const;
const linux = { os: "linux", arch: "x86_64" } as const;
const relocatable = { kind: "relocatable" } as const;

function built(name = "tool", target: WorkspacePlatform = linux, layout: PackageLayout = relocatable) {
  const source = pkg(`${name}-src`, {
    producer: provider(tarball(`https://example.invalid/${name}.tar.gz`)),
    commands: {}, layout,
    target,
  });
  const packageValue = pkg(name, {
    producer: provider(tarball(`https://example.invalid/${name}.tar.gz`)),
    commands: { tool: "bin/tool" },
    target,
    layout,
  });
  return { source, packageValue };
}

function emit(outputs: WorkspaceOutput[]): string {
  return emitWorkspaceIr(workspace({ outputs }), facts);
}

Deno.test("artifact refs require an actual artifact with both sites reported", () => {
  const noArtifact = task("ephemeral", { steps: [exec({ argv: [lit("true")] })] });
  const config = profile("config", {
    files: [file({
      source: artifactFile("ephemeral", "config"),
      content: identity(),
      destination: trackedCopyTo("~/.config/tool"),
    })],
  });
  const diagnostic = thrownDiagnostic(
    () => emit([noArtifact, config]),
    "E126",
  );
  assert.equal(diagnostic.labels.length, 2, "reference and declaration spans labeled");
  assert.ok(diagnostic.labels[0]?.span?.line !== diagnostic.labels[1]?.span?.line);
});

Deno.test("check subjects resolve rather than silently becoming opaque names", () => {
  const missing = check("validate", { run: exec({ argv: [lit("true")] }), subject: "ghost" });
  const subjectFailure = thrownDiagnostic(() => emit([missing]), "E126");
  assert.equal(subjectFailure.labels[0]?.span?.line, missing.ir.span.line);
  const valid = check("validate", { run: exec({ argv: [lit("true")] }), subject: "validate" });
  assert.equal(JSON.parse(emit([valid])).workspace.outputs[0].subject, "validate");
});

Deno.test("artifact selectors cannot escape or use non-canonical path segments", () => {
  for (const path of ["../secret", "/etc/passwd", "bin//tool", "./tool", "bin/../tool", "\0"]) {
    assert.throws(() => artifact("tool", path), /selector must be/);
    assert.throws(() => artifactFile("tool", path), /selector must be/);
  }
  const { source, packageValue } = built();
  const canonical = task("use", { steps: [exec({ argv: [artifact("tool", "bin/tool")] })] });
  assert.equal(JSON.parse(emit([source, packageValue, canonical])).workspace.outputs.length, 3);
  // Hand-built values bypass constructor guards; the emitter still rejects them with the site.
  const fake = {
    __gripsack: "workspace",
    ir: { span: { file: "direct.ts", line: 1 }, outputs: [source.ir, packageValue.ir, {
      ...canonical.ir,
      steps: [{ command: { kind: "exec", span: { file: "direct.ts", line: 7 }, argv: [
        { kind: "artifact", output: "tool", selector: "../secret" },
      ] } }],
    }] },
  } as unknown as WorkspaceValue;
  const escaping = thrownDiagnostic(() => emitWorkspaceIr(fake, facts), "E130");
  assert.deepEqual(escaping.labels[0]?.span, { file: "direct.ts", line: 7 });
});

Deno.test("environment values are data, never package commands", () => {
  assert.throws(
    () => exec({ argv: [lit("env")], env: { TOOL: packageCommand("tool", "tool") } }),
    /cannot be a package_command reference/,
  );
  assert.throws(
    () => environment("dev", { packages: [], target: linux, env: { TOOL: packageCommand("tool", "tool") } }),
    /cannot be a package_command reference/,
  );
});

Deno.test("fixed-prefix selection needs a matching environment location", () => {
  const { source, packageValue } = built("pinned", linux, { kind: "fixed_prefix", prefix: "/opt/tool" });
  assert.equal(JSON.parse(emit([source, packageValue])).workspace.outputs.length, 2);
  const env = environment("dev", { packages: ["pinned"], target: linux });
  const prefixless = thrownDiagnostic(() => emit([source, packageValue, env]), "E126");
  assert.equal(prefixless.labels.length, 2, "environment and package spans labeled");
  const matching = environment("dev", { packages: ["pinned"], target: linux, prefix: "/opt/tool" });
  assert.equal(JSON.parse(emit([source, packageValue, matching])).workspace.outputs.length, 3);
  const wrong = environment("dev", { packages: ["pinned"], target: linux, prefix: "/other" });
  const wrongPrefix = thrownDiagnostic(() => emit([source, packageValue, wrong]), "E126");
  assert.equal(wrongPrefix.labels.length, 2);
});

Deno.test("ABI mismatch and incompatible minimum OS reject with both declarations", () => {
  const producer = { ...linux, abi: "gnu", minimum_os: { major: 5, minor: 15 } } as const;
  const consumer = { ...linux, abi: "gnu", minimum_os: { major: 6, minor: 1 } } as const;
  const { source, packageValue } = built("kernel-tool", producer);
  const supported = environment("dev", { packages: ["kernel-tool"], target: consumer });
  assert.equal(JSON.parse(emit([source, packageValue, supported])).workspace.outputs.length, 3);
  const older = environment("dev", { packages: ["kernel-tool"], target: { ...linux, abi: "gnu", minimum_os: { major: 4, minor: 19 } } });
  const olderRejected = thrownDiagnostic(() => emit([source, packageValue, older]), "E126");
  assert.equal(olderRejected.labels.length, 2, "consumer and provider spans labeled");
  const unspecified = environment("dev", { packages: ["kernel-tool"], target: linux });
  const missingFloor = thrownDiagnostic(() => emit([source, packageValue, unspecified]), "E126");
  assert.equal(missingFloor.labels.length, 2);
  for (const bad of ["/", "relative/path", "/opt/../home", "/opt//tool", "/opt/tool/", "/opt/\0tool"]) {
    assert.throws(() => built("bad-prefix", linux, { kind: "fixed_prefix", prefix: bad }), /normalized absolute POSIX install path/);
  }
});

Deno.test("an ABI-agnostic environment admits abi-specific members; a declared ABI stays exact", () => {
  const { source, packageValue } = built("nvim", { ...linux, abi: "gnu" });
  const agnostic = environment("tools", { packages: ["nvim"], target: linux });
  assert.equal(JSON.parse(emit([source, packageValue, agnostic])).workspace.outputs.length, 3);
  const gnu = environment("tools", { packages: ["nvim"], target: { ...linux, abi: "gnu" } });
  assert.equal(JSON.parse(emit([source, packageValue, gnu])).workspace.outputs.length, 3);
  const musl = environment("tools", { packages: ["nvim"], target: { ...linux, abi: "musl" } });
  thrownDiagnostic(() => emit([source, packageValue, musl]), "E126");
  // The reverse stays strict: an agnostic provider cannot satisfy a declared ABI.
  const plain = built("plain", linux);
  const wantsGnu = environment("tools", { packages: ["plain"], target: { ...linux, abi: "gnu" } });
  thrownDiagnostic(() => emit([plain.source, plain.packageValue, wantsGnu]), "E126");
});

function assertEmitsUnchanged(outputs: WorkspaceOutput[]): void {
  const value = workspace({ outputs });
  assert.deepEqual(JSON.parse(emitWorkspaceIr(value, facts)).workspace, value.ir);
}

Deno.test("source and output bindings require an owner in argv, cwd and environment data", () => {
  const { packageValue } = built();
  const runSpan = { file: "checks.ts", line: 11 };
  for (const binding of [sourcePath("bin"), outputPath(".")]) {
    const env = environment("dev", { target: linux, packages: [], env: { ROOT: binding } });
    const error = thrownDiagnostic(() => emit([env]), "E128");
    assert.deepEqual(error.labels.map((label) => label.span), [env.ir.span]);
    for (const run of [
      exec({ argv: [lit("test"), binding], span: runSpan }),
      exec({ argv: [lit("test")], cwd: binding, span: runSpan }),
      exec({ argv: [lit("test")], env: { ROOT: binding }, span: runSpan }),
    ]) {
      const taskValue = task("job", { steps: [run] });
      const hookValue = hook("trigger", { run, trigger: "post_link" });
      const nonArtifactCheck = check("observe", { subject: "observe", run });
      for (const invocation of [taskValue, hookValue, nonArtifactCheck]) {
        const rejected = thrownDiagnostic(() => emit([invocation]), "E128");
        assert.deepEqual(rejected.labels.map((label) => label.span), [runSpan]);
      }
      const pkgCheck = check("observe", { subject: packageValue.ir.name, run });
      if (binding.kind === "output") {
        const rejected = thrownDiagnostic(() => emit([pkgCheck, packageValue]), "E128");
        assert.deepEqual(rejected.labels[0]?.span, runSpan);
      } else {
        assertEmitsUnchanged([pkgCheck, packageValue]);
      }
    }
  }
});

Deno.test("only the attached file itself supplies check data, never cwd", () => {
  const home = profile("home", { files: [file({
    content: literalText("data"), destination: trackedCopyTo("~/config"),
    checks: [{ check: "syntax", subject: "rendered", stage: "pre_flip" }],
  })] });
  const root = sourcePath(".");
  const valid = exec({ argv: [lit("test"), root], env: { SUBJECT: root } });
  const bound = check("syntax", { subject: "home", run: valid });
  assertEmitsUnchanged([bound, home]);
  for (const run of [
    exec({ argv: [lit("test")], env: { SUBJECT: sourcePath("other") } }),
    exec({ argv: [lit("test")], cwd: root }),
  ]) {
    const rejected = thrownDiagnostic(() => emit([home, check("syntax", { subject: "home", run })]), "E128");
    assert.deepEqual(rejected.labels[0]?.span, run.span);
  }
});

Deno.test("runtime and selected packages require compatible targets; passive artifact data does not", () => {
  const target = { ...linux, abi: "gnu", minimum_os: { major: 6, minor: 1, patch: 2 } } as const;
  const { packageValue } = built("lib", target);
  for (const [consumerTarget, ok] of [
    [{ ...target, minimum_os: { major: 6, minor: 2 } }, true],
    [{ ...target, minimum_os: { major: 6, minor: 1 } }, false],
    [{ ...target, abi: "musl" }, false],
  ] as const) {
    const runtime = pkg("tool", {
      producer: provider(tarball("https://example.invalid/tool.tgz")), commands: {},
      target: consumerTarget, layout: relocatable, runtime: ["lib"],
    });
    const data = environment("dev", {
      packages: [], env: { LIB: artifact("lib", ".") }, target: consumerTarget,
    });
    assertEmitsUnchanged([data, packageValue]);
    const selected = environment("run", { packages: ["lib"], target: consumerTarget });
    for (const consumer of [runtime, selected]) {
      if (ok) assertEmitsUnchanged([consumer, packageValue]);
      else {
        const mismatch = thrownDiagnostic(() => emit([consumer, packageValue]), "E126");
        assert.deepEqual(mismatch.labels.map((label) => label.span), [consumer.ir.span, packageValue.ir.span]);
      }
    }
  }
});

Deno.test("producer layout matrix preserves imported layouts and derived Conda/Pixi prefixes", () => {
  const inputs = [inputFile("manifest", "pixi.toml"), inputFile("lock", "pixi.lock")];
  const layouts: PackageLayout[] = [relocatable, { kind: "fixed_prefix", prefix: "/opt/tools" }, { kind: "prefix_materialized" }];
  for (const [producer, accepted] of [
    [provider(tarball("https://example.invalid/tool.tgz")), [true, true, false]],
    [provider(conda.environment({ channels: ["conda-forge"], packages: { python: "*" } })), [false, false, true]],
    [provider(pixi.fromLock({ manifest: "manifest", lock: "lock", environment: "default" })), [false, false, true]],
    [provider(source.package({ manifest: "a".repeat(64) })), [true, true, true]],
  ] as const) {
    for (const [index, layout] of layouts.entries()) {
      const value = pkg("tools", { producer, commands: {}, target: linux, layout });
      const compile = (...rest: WorkspaceOutput[]) => emitWorkspaceIr(workspace({ outputs: [value, ...rest], inputs }), facts);
      if (!accepted[index]) {
        assert.deepEqual(thrownDiagnostic(() => compile(), "E130").labels[0]?.span, value.ir.span);
        continue;
      }
      assert.deepEqual(JSON.parse(compile()).workspace.outputs[0].layout, layout);
      for (const selection of [{ packages: ["tools"] }, { packages: [], env: { TOOL: artifact("tools", ".") } }]) {
        for (const prefix of [undefined, "/opt/tools", "/other"]) {
          const env = environment("dev", { ...selection, target: linux, ...(prefix === undefined ? {} : { prefix }) });
          const valid = layout.kind === "relocatable" ||
            (layout.kind === "prefix_materialized" ? prefix === undefined : prefix === layout.prefix);
          if (valid) assert.deepEqual(JSON.parse(compile(env)).workspace.outputs, [env.ir, value.ir]);
          else {
            const mismatch = thrownDiagnostic(() => compile(env), "E126");
            assert.deepEqual(mismatch.labels.map((label) => label.span), [env.ir.span, value.ir.span]);
          }
        }
      }
    }
  }
});

Deno.test("literal destination ownership spans profiles, but distinct blocks and tree roots coexist", () => {
  function declaration(name: string, line: number, destination: WorkspaceDestination) {
    return profile(name, { files: [file({ content: literalText(name), destination, span: { file: "files.ts", line } })] });
  }
  const first = declaration("one", 7, trackedCopyTo("~/config"));
  const second = declaration("two", 8, symlinkTo("~/CONFIG"));
  const block = declaration("block", 9, managedBlock("~/config", "Tool"));
  const other = declaration("other", 10, managedBlock("~/CONFIG", "Other"));
  const duplicate = declaration("duplicate", 11, managedBlock("~/CONFIG", "tool"));
  for (const [owners, lines] of [
    [[first, second], [7, 8]], [[block, first, other], [7, 9, 10]],
    [[other, duplicate, block], [9, 11]],
  ] as const) {
    const diagnostic = thrownDiagnostic(() => emit([...owners]), "E111");
    assert.deepEqual(diagnostic.labels.map((label) => label.span?.line), lines);
    assert.ok(diagnostic.help);
  }
  const { packageValue } = built();
  const trees = profile("trees", { files: [
    file({ source: repoTree("configs"), content: identity(), destination: trackedCopyTo("~/config") }),
    file({ source: artifactTree("tool", { include: ["etc"] }), content: identity(), destination: symlinkTo("~/config") }),
  ] });
  assertEmitsUnchanged([block, other, trees, packageValue]);
});
