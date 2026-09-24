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
  assert.throws(
    () => emit([noArtifact, config]),
    /expects 'ephemeral' to be one of 'recipe', 'package'.*referenced at .*:\d+, declared at .*:\d+/,
  );
  const img = image("img", { packages: [], target: linux });
  const consumer = task("consume", { run: exec({ argv: [artifact("img", "manifest")] }) });
  assert.throws(() => emit([img, consumer]), /expects 'img' to be one of 'recipe', 'package'/);
});

Deno.test("check subjects resolve rather than silently becoming opaque names", () => {
  const missing = check("validate", { run: exec({ argv: [lit("true")] }), subject: "ghost" });
  assert.throws(() => emit([missing]), /output 'validate' references unknown output 'ghost'/);
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
  assert.throws(() => emitWorkspaceIr(fake, facts), /selector must be.*referenced at direct.ts:7/);
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
  assert.throws(() => emit([source, mismatched]), /producer target mismatch.*declared at .*:\d+.*declared at .*:\d+/);
  const env = environment("dev", { packages: ["mac-tool"], target: linux });
  assert.throws(() => emit([source, packageValue, env]), /selection target mismatch.*declared at .*:\d+.*declared at .*:\d+/);
  const correct = environment("dev", { packages: ["mac-tool"], target: mac });
  assert.equal(JSON.parse(emit([source, packageValue, correct])).workspace.outputs.length, 3);
});

Deno.test("fixed-prefix selection needs a matching environment location", () => {
  const { source, packageValue } = built("pinned", linux, { kind: "fixed_prefix", prefix: "/opt/tool" });
  assert.equal(JSON.parse(emit([source, packageValue])).workspace.outputs.length, 2);
  const env = environment("dev", { packages: ["pinned"], target: linux });
  assert.throws(() => emit([source, packageValue, env]), /fixed_prefix.*no matching install prefix.*declared at .*:\d+.*declared at .*:\d+/);
  const matching = environment("dev", { packages: ["pinned"], target: linux, prefix: "/opt/tool" });
  assert.equal(JSON.parse(emit([source, packageValue, matching])).workspace.outputs.length, 3);
  const wrong = environment("dev", { packages: ["pinned"], target: linux, prefix: "/other" });
  assert.throws(() => emit([source, packageValue, wrong]), /fixed_prefix.*no matching install prefix/);
  const img = image("container", { packages: ["pinned"], target: linux });
  assert.throws(() => emit([source, packageValue, img]), /fixed_prefix.*no matching install prefix/);
});

Deno.test("ABI mismatch and incompatible minimum OS reject with both declarations", () => {
  const producer = { ...linux, abi: "gnu", minimum_os: { major: 5, minor: 15 } } as const;
  const consumer = { ...linux, abi: "gnu", minimum_os: { major: 6, minor: 1 } } as const;
  const { source, packageValue } = built("kernel-tool", producer);
  const supported = environment("dev", { packages: ["kernel-tool"], target: consumer });
  assert.equal(JSON.parse(emit([source, packageValue, supported])).workspace.outputs.length, 3);
  const older = environment("dev", { packages: ["kernel-tool"], target: { ...linux, abi: "gnu", minimum_os: { major: 4, minor: 19 } } });
  assert.throws(() => emit([source, packageValue, older]), /selection target mismatch.*declared at .*:\d+.*declared at .*:\d+/);
  const unspecified = environment("dev", { packages: ["kernel-tool"], target: linux });
  assert.throws(() => emit([source, packageValue, unspecified]), /selection target mismatch/);
  assert.throws(() => built("bad-abi", { ...linux, abi: "darwin" }), /abi is incompatible with linux/);
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
