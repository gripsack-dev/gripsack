import assert from "node:assert/strict";
import { tarball } from "../src/fetch.ts";
import {
  artifact,
  artifactFile,
  check,
  emitWorkspaceIr,
  environment,
  exec,
  file,
  identity,
  image,
  lit,
  packageCommand,
  pkg,
  profile,
  recipe,
  task,
  trackedCopyTo,
  workspace,
} from "../src/workspace.ts";
import type { PackageLayout, WorkspaceOutput, WorkspacePlatform, WorkspaceValue } from "../src/workspace.ts";
import { thrownDiagnostic } from "./diagnostic.ts";

const facts = { os: "linux", arch: "x86_64", libc: null, hostname: "builder" } as const;
const linux = { os: "linux", arch: "x86_64" } as const;
const mac = { os: "macos", arch: "aarch64" } as const;
const hostExecution = { kind: "host", access: "unconfined" } as const;
const relocatable = { kind: "relocatable" } as const;

function built(name = "tool", target: WorkspacePlatform = linux, layout: PackageLayout = relocatable) {
  const source = recipe(`${name}-src`, {
    source: tarball(`https://example.invalid/${name}.tar.gz`),
    execution: hostExecution,
    output_kind: "tree",
    target,
  });
  const packageValue = pkg(name, {
    producer: `${name}-src`,
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
  const noArtifact = task("ephemeral", { run: exec({ argv: [lit("true")] }) });
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
  const img = image("img", { packages: [], target: linux });
  const consumer = task("consume", { run: exec({ argv: [artifact("img", "manifest")] }) });
  const imageFailure = thrownDiagnostic(() => emit([img, consumer]), "E126");
  assert.deepEqual(imageFailure.labels.map((label) => label.span?.line), [
    consumer.ir.span.line, img.ir.span.line,
  ]);
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
  const canonical = task("use", { run: exec({ argv: [artifact("tool", "bin/tool")] }) });
  assert.equal(JSON.parse(emit([source, packageValue, canonical])).workspace.outputs.length, 3);
  // Hand-built values bypass constructor guards; the emitter still rejects them with the site.
  const fake = {
    __gripsack: "workspace",
    ir: { span: { file: "direct.ts", line: 1 }, outputs: [source.ir, packageValue.ir, {
      ...canonical.ir,
      run: { kind: "exec", span: { file: "direct.ts", line: 7 }, argv: [
        { kind: "artifact", output: "tool", selector: "../secret" },
      ] },
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

Deno.test("targets bind ABI and minimum OS requirements, not the current host", () => {
  const { source, packageValue } = built("mac-tool", mac);
  assert.equal(JSON.parse(emit([source, packageValue])).workspace.outputs.length, 2);
  const mismatched = pkg("wrong", {
    producer: "mac-tool-src", commands: { tool: "bin/tool" }, target: linux,
    layout: relocatable,
  });
  const producerMismatch = thrownDiagnostic(() => emit([source, mismatched]), "E126");
  assert.equal(producerMismatch.labels.length, 2, "consumer and provider spans labeled");
  const env = environment("dev", { packages: ["mac-tool"], target: linux });
  const selectionMismatch = thrownDiagnostic(() => emit([source, packageValue, env]), "E126");
  assert.equal(selectionMismatch.labels.length, 2, "consumer and provider spans labeled");
  const correct = environment("dev", { packages: ["mac-tool"], target: mac });
  assert.equal(JSON.parse(emit([source, packageValue, correct])).workspace.outputs.length, 3);
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
  const img = image("container", { packages: ["pinned"], target: linux });
  const imagePrefix = thrownDiagnostic(() => emit([source, packageValue, img]), "E126");
  assert.equal(imagePrefix.labels.length, 2);
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

Deno.test("recipe command lists retain local order without task-prerequisite edges", () => {
  const first = exec({ argv: [lit("first")], span: { file: "recipe.ts", line: 4 } });
  const second = exec({ argv: [lit("second")], span: { file: "recipe.ts", line: 5 } });
  const source = tarball("https://example.invalid/order.tar.gz");
  const declared = (steps: typeof first[]) => recipe("order", {
    source, execution: hostExecution, output_kind: "tree", target: linux,
    steps, span: { file: "recipe.ts", line: 2 },
  });
  const commands = (steps: typeof first[]) =>
    JSON.parse(emit([declared(steps)])).workspace.outputs[0].steps.map(
      (command: { argv: { value: string }[] }) => command.argv[0].value,
    );
  assert.deepEqual(commands([first, second]), ["first", "second"]);
  assert.deepEqual(commands([second, first]), ["second", "first"]);
});
