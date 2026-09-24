/** Workspace contract tests (0052 A1): the v4 workspace envelope,
 *  the nine typed output constructors, run_bash literal/interpreter
 *  boundaries with dedent source maps, typed reference/cycle
 *  validation, and the no-registry purity guarantee. */

import assert from "node:assert/strict";
import { emitIr, module } from "../src/index.ts";
import {
  artifact,
  artifactFile,
  check,
  daily,
  emitWorkspaceIr,
  environment,
  exec,
  file,
  hook,
  hostPath,
  identity,
  image,
  lit,
  literalText,
  managedBlock,
  packageCommand,
  pkg,
  profile,
  provider,
  recipe,
  repoFile,
  runBash,
  schedule,
  symlinkTo,
  targetPlatform,
  task,
  templateText,
  trackedCopyTo,
  weekly,
  workspace,
} from "../src/workspace.ts";
import type { WorkspaceValue } from "../src/workspace.ts";
import { githubRelease, tarball } from "../src/fetch.ts";
import type { HostFacts } from "../src/index.ts";

const facts: HostFacts = { os: "linux", arch: "x86_64", libc: "glibc-2.36", hostname: "box" };
const linux = { os: "linux", arch: "x86_64" } as const;

const emit = (value: WorkspaceValue, tags: string[] = []) =>
  JSON.parse(emitWorkspaceIr(value, facts, tags));

/** The full kitchen sink: every output kind wired through typed refs. */
function kitchenSink(): WorkspaceValue {
  const bashSrc = recipe("bash-src", {
    source: tarball("https://example.invalid/bash.tar.gz"),
    execution: "native",
    output_kind: "tree",
    target: linux,
  });
  const bash = pkg("bash", {
    producer: "bash-src",
    commands: { bash: "bin/bash" },
    target: linux,
    layout: "relocatable",
  });
  const tools = recipe("tools", {
    source: githubRelease({ repo: "example/tools", asset: "tools-{version}.tar.gz" }),
    execution: "host",
    output_kind: "tree",
    target: linux,
    steps: [
      runBash({
        interpreter: packageCommand("bash", "bash"),
        body: "make install",
        env: { PREFIX: lit("/usr") },
        cwd: hostPath("/tmp"),
      }),
      exec({ argv: [lit("cp"), artifact("bash", "share/doc"), lit("share/doc")] }),
    ],
    checks: ["tools-ok"],
  });
  const toolsBin = pkg("tools-bin", {
    producer: "tools",
    commands: { tools: "bin/tools" },
    runtime: ["bash"],
    target: linux,
    layout: "fixed_prefix",
  });
  const toolsOk = check("tools-ok", {
    run: exec({ argv: [packageCommand("tools-bin", "tools"), lit("--version")] }),
    subject: "tools-bin",
  });
  // Explicit environment/image package selections cannot place a
  // fixed-prefix package. Other fixed-prefix references stay
  // descriptive while executor/prefix policy remains A1-02/A2-04.
  const dev = environment("dev", {
    packages: ["bash"],
    target: linux,
    env: { TOOLS_HOME: artifact("tools-bin", ".") },
  });
  const build = task("build", {
    run: exec({
      argv: [packageCommand("tools-bin", "tools"), lit("build")],
      cwd: { kind: "literal", value: "." },
    }),
    deps: ["lint"],
    environment: "dev",
    checks: ["tools-ok"],
  });
  const lint = task("lint", {
    run: exec({ argv: [packageCommand("tools-bin", "tools"), lit("lint")] }),
    environment: "dev",
  });
  const nightly = schedule("nightly", { task: "build", trigger: weekly("fri", "03:30") });
  const reload = hook("reload", {
    run: exec({ argv: [lit("true")] }),
    trigger: "post_activate",
  });
  const me = profile("me", {
    files: [
      file({ content: literalText("x=1\n"), destination: symlinkTo("~/.xrc") }),
      file({
        source: repoFile("dotfiles/bashrc.sh"),
        content: identity(),
        destination: managedBlock("~/.bashrc", "#"),
      }),
      file({
        source: artifactFile("tools-bin", "share/tools.toml"),
        content: templateText("editor = {{ editor }}", { editor: "hx" }),
        destination: trackedCopyTo("~/.config/tools.toml"),
      }),
    ],
    environment: "dev",
    schedules: ["nightly"],
    hooks: ["reload"],
  });
  const ci = image("ci", { packages: ["bash"], target: linux });
  return workspace({
    outputs: [
      bashSrc,
      bash,
      tools,
      toolsBin,
      toolsOk,
      dev,
      build,
      lint,
      nightly,
      reload,
      me,
      ci,
    ],
  });
}

Deno.test("emitWorkspaceIr emits the v4 workspace envelope", () => {
  const ir = emit(kitchenSink(), ["work"]);

  assert.deepEqual(Object.keys(ir), ["ir_version", "host", "workspace"]);
  assert.equal(ir.ir_version, 4);
  assert.equal(ir.modules, undefined, "workspace envelope never carries modules");
  assert.equal(ir.resources, undefined);
  assert.deepEqual(Object.keys(ir.host), ["os", "arch", "tags", "libc"]);
  assert.equal("hostname" in ir.host, false, "hostname never crosses into the IR");
  assert.deepEqual(ir.host.tags, ["work"]);

  assert.deepEqual(Object.keys(ir.workspace), ["span", "outputs"]);
  assert.match(ir.workspace.span.file, /workspace\.test\.ts$/);
  assert.ok(ir.workspace.span.line >= 1);

  const byName = Object.fromEntries(
    ir.workspace.outputs.map((o: { name: string }) => [o.name, o]),
  );
  assert.deepEqual(Object.keys(byName), [
    "bash-src",
    "bash",
    "tools",
    "tools-bin",
    "tools-ok",
    "dev",
    "build",
    "lint",
    "nightly",
    "reload",
    "me",
    "ci",
  ]);

  // every output carries name/kind/span with a real provenance
  for (const o of ir.workspace.outputs) {
    assert.ok(o.name.length > 0);
    assert.match(o.span.file, /workspace\.test\.ts$/);
    assert.ok(o.span.line >= 1, `${o.name} span.line`);
  }
  assert.deepEqual(
    ir.workspace.outputs.map((o: { kind: string }) => o.kind),
    [
      "recipe",
      "package",
      "recipe",
      "package",
      "check",
      "environment",
      "task",
      "task",
      "schedule",
      "hook",
      "profile",
      "image",
    ],
  );

  // recipe: workspaceFetch wrapper carries its own mandatory span
  assert.deepEqual(Object.keys(byName.tools.source), ["fetch", "span"]);
  assert.equal(byName.tools.source.fetch.kind, "github_release");
  assert.ok(byName.tools.source.span.line >= 1);
  assert.equal(byName.tools.execution, "host");
  assert.deepEqual(byName.tools.checks, ["tools-ok"]);

  // run_bash: pinned package_command interpreter, literal body, no line_map
  const rb = byName.tools.steps[0];
  assert.equal(rb.kind, "run_bash");
  assert.deepEqual(rb.interpreter, { kind: "package_command", package: "bash", command: "bash" });
  assert.equal(rb.body, "make install");
  assert.equal(rb.line_map, undefined, "single-line body emits no line_map");
  assert.deepEqual(rb.env, { PREFIX: { kind: "literal", value: "/usr" } });
  assert.deepEqual(rb.cwd, { kind: "host", path: "/tmp" });
  assert.ok(rb.span.line >= 1, "command span");

  // exec: typed argv refs
  const ex = byName.tools.steps[1];
  assert.deepEqual(ex.argv, [
    { kind: "literal", value: "cp" },
    { kind: "artifact", output: "bash", selector: "share/doc" },
    { kind: "literal", value: "share/doc" },
  ]);

  // package: recipe-ref producer in the tagged wire form
  assert.deepEqual(byName["tools-bin"].producer, { kind: "recipe", recipe: "tools" });
  assert.deepEqual(byName["tools-bin"].commands, { tools: "bin/tools" });
  assert.deepEqual(byName["tools-bin"].runtime, ["bash"]);
  assert.equal(byName["tools-bin"].layout, "fixed_prefix");

  // task / schedule / hook
  assert.deepEqual(byName.build.deps, ["lint"]);
  assert.equal(byName.build.environment, "dev");
  assert.deepEqual(byName.nightly.trigger, { kind: "weekly", weekday: "fri", time: "03:30" });
  assert.equal(byName.nightly.scope, "user");
  assert.equal(byName.reload.trigger, "post_activate");

  // profile files: origin optional only for literal content
  const [rc, bashrc, toolsToml] = byName.me.files;
  assert.equal(rc.source, undefined, "literal content needs no origin");
  assert.deepEqual(rc.content, { kind: "literal", text: "x=1\n" });
  assert.deepEqual(rc.destination, { kind: "symlink", path: "~/.xrc" });
  assert.deepEqual(bashrc.source, { kind: "repo_file", path: "dotfiles/bashrc.sh" });
  assert.deepEqual(bashrc.content, { kind: "identity" });
  assert.deepEqual(bashrc.destination, { kind: "managed_block", path: "~/.bashrc", marker: "#" });
  assert.deepEqual(toolsToml.source, {
    kind: "artifact_file",
    output: "tools-bin",
    selector: "share/tools.toml",
  });
  assert.deepEqual(toolsToml.content, {
    kind: "template",
    template: "editor = {{ editor }}",
    variables: { editor: "hx" },
  });
  assert.ok(rc.span.line >= 1, "file span");
  assert.deepEqual(byName.me.schedules, ["nightly"]);
  assert.deepEqual(byName.me.hooks, ["reload"]);

  // image / environment / check
  assert.deepEqual(byName.ci.packages, ["bash"]);
  assert.deepEqual(byName.dev.packages, ["bash"]);
  assert.deepEqual(byName.dev.env, {
    TOOLS_HOME: { kind: "artifact", output: "tools-bin", selector: "." },
  });
  assert.equal(byName["tools-ok"].subject, "tools-bin");
});

Deno.test("emitIr legacy path emits the v4 modules envelope, never a workspace", () => {
  const ir = JSON.parse(emitIr({ modules: [module("m", { lint: "toml" })] }, facts, []));
  assert.deepEqual(Object.keys(ir), ["ir_version", "host", "modules"]);
  assert.equal(ir.ir_version, 4);
  assert.equal(ir.workspace, undefined);
  assert.equal(ir.modules.m.lint, "toml");
});

Deno.test("duplicate output names throw at declaration with both sites", () => {
  const a = recipe("same", {
    source: tarball("https://example.invalid/a.tar.gz"),
    execution: "native",
    output_kind: "file",
    target: linux,
  });
  const b = image("same", { packages: [], target: linux });
  assert.throws(
    () => workspace({ outputs: [a, b] }),
    /duplicate output 'same' \(first declared at .*workspace\.test\.ts:\d+, again at .*workspace\.test\.ts:\d+\)/,
  );
});

Deno.test("duplicate output names throw at emit for hand-built values", () => {
  const a = recipe("same", {
    source: tarball("https://example.invalid/a.tar.gz"),
    execution: "native",
    output_kind: "file",
    target: linux,
  });
  const node = JSON.parse(JSON.stringify(a.ir));
  const fake = {
    __gripsack: "workspace",
    ir: { span: { file: "fake.ts", line: 1 }, outputs: [node, node] },
  } as unknown as WorkspaceValue;
  assert.throws(
    () => emitWorkspaceIr(fake, facts),
    /duplicate output 'same' \(first declared at .*:\d+, again at .*:\d+\)/,
  );
});

Deno.test("constructors reject unknown fields for JS callers and casts", () => {
  assert.throws(
    () =>
      recipe("x", {
        soruce: tarball("https://example.invalid/x.tar.gz"),
        execution: "native",
        output_kind: "file",
        target: linux,
      } as never),
    /unknown field "soruce".*did you mean "source"\?/,
  );
  assert.throws(
    () => exec({ argvv: [] } as never),
    /unknown field "argvv".*did you mean "argv"\?/,
  );
  assert.throws(
    () =>
      file({
        content: literalText("x"),
        destination: symlinkTo("~/.x"),
        sourc: repoFile("a"),
      } as never),
    /unknown field "sourc".*did you mean "source"\?/,
  );
  assert.throws(
    () => schedule("s", { task: "build", triger: daily("01:00") } as never),
    /unknown field "triger".*did you mean "trigger"\?/,
  );
});

Deno.test("run_bash rejects interpolation at the declaration span", () => {
  assert.throws(
    () =>
      runBash({
        interpreter: packageCommand("bash", "bash"),
        body: "echo ${HOME} && true",
      }),
    /literal text — "\$\{" interpolation is rejected \(declared at .*workspace\.test\.ts:\d+\)/,
  );
});

Deno.test("run_bash requires a pinned package_command interpreter", () => {
  assert.throws(
    () => runBash({ interpreter: lit("bash"), body: "true" }),
    /interpreter must be a pinned packageCommand\("<package>", "<command>"\)/,
  );
  assert.throws(
    () => runBash({ interpreter: artifact("tools", "bin/bash"), body: "true" }),
    /interpreter must be a pinned packageCommand/,
  );
});

Deno.test("run_bash dedents multi-line bodies with a source line_map", () => {
  const cmd = runBash({
    interpreter: packageCommand("bash", "bash"),
    body: `
      echo one
      echo two
    `,
  });
  assert.equal(cmd.kind, "run_bash");
  assert.equal(cmd.body, "echo one\necho two");
  assert.deepEqual(cmd.line_map, [cmd.span.line + 1, cmd.span.line + 2]);
  assert.match(cmd.span.file, /workspace\.test\.ts$/);
});

Deno.test("file origin is optional only for literal content", () => {
  assert.throws(
    () => file({ content: identity(), destination: symlinkTo("~/.x") }),
    /source is required unless content is literalText/,
  );
  assert.throws(
    () =>
      file({
        content: templateText("{{ v }}", { v: "1" }),
        destination: symlinkTo("~/.x"),
      }),
    /source is required unless content is literalText/,
  );
  const literal = file({ content: literalText("x"), destination: symlinkTo("~/.x") });
  assert.equal(literal.source, undefined);
  const templated = file({
    source: repoFile("t/x.tmpl"),
    content: templateText("{{ v }}", { v: "1" }),
    destination: trackedCopyTo("~/.x"),
  });
  assert.equal(templated.source?.kind, "repo_file");
});

Deno.test("emit rejects unknown output references with the reference span", () => {
  const t = task("build", {
    run: exec({ argv: [lit("true")] }),
    deps: ["ghost"],
  });
  assert.throws(
    () => emitWorkspaceIr(workspace({ outputs: [t] }), facts),
    /output 'build' references unknown output 'ghost' \(referenced at .*workspace\.test\.ts:\d+\)/,
  );
});

Deno.test("emit rejects wrong-kind references naming both sites", () => {
  const dev = environment("dev", { packages: [], target: linux });
  const p = pkg("p", {
    producer: "dev",
    commands: { p: "bin/p" },
    target: linux,
    layout: "relocatable",
  });
  assert.throws(
    () => emitWorkspaceIr(workspace({ outputs: [dev, p] }), facts),
    /output 'p' expects 'dev' to be a recipe — it is a 'environment' \(referenced at .*:\d+, declared at .*:\d+\)/,
  );
});

Deno.test("emit rejects package_command refs to unknown commands", () => {
  const src = recipe("src", {
    source: tarball("https://example.invalid/x.tar.gz"),
    execution: "native",
    output_kind: "tree",
    target: linux,
  });
  const p = pkg("p", {
    producer: "src",
    commands: { tool: "bin/tool" },
    target: linux,
    layout: "relocatable",
  });
  const c = check("c", {
    run: exec({ argv: [packageCommand("p", "nope")] }),
    subject: "p",
  });
  assert.throws(
    () => emitWorkspaceIr(workspace({ outputs: [src, p, c] }), facts),
    /package 'p' has no command 'nope' .*; it provides: tool/,
  );
});

Deno.test("emit rejects task dependency cycles", () => {
  const a = task("a", { run: exec({ argv: [lit("true")] }), deps: ["b"] });
  const b = task("b", { run: exec({ argv: [lit("true")] }), deps: ["a"] });
  assert.throws(
    () => emitWorkspaceIr(workspace({ outputs: [a, b] }), facts),
    /dependency cycle a -> b -> a .*'a' declared at .*:\d+.*'b' declared at .*:\d+/,
  );
});

Deno.test("emit rejects producer/artifact cycles but admits validation loops", () => {
  // recipe artifact-refs the package it produces: a real cycle
  const cyc = recipe("cyc", {
    source: tarball("https://example.invalid/x.tar.gz"),
    execution: "native",
    output_kind: "tree",
    target: linux,
    steps: [exec({ argv: [artifact("p", "bin/p")] })],
  });
  const p = pkg("p", {
    producer: "cyc",
    commands: { p: "bin/p" },
    target: linux,
    layout: "relocatable",
  });
  assert.throws(
    () => emitWorkspaceIr(workspace({ outputs: [cyc, p] }), facts),
    /dependency cycle/,
  );

  // a recipe gated by a check on the package it produces is legitimate
  const tools = recipe("tools", {
    source: tarball("https://example.invalid/t.tar.gz"),
    execution: "native",
    output_kind: "tree",
    target: linux,
    checks: ["ok"],
  });
  const bin = pkg("bin", {
    producer: "tools",
    commands: { t: "bin/t" },
    target: linux,
    layout: "relocatable",
  });
  const ok = check("ok", { run: exec({ argv: [lit("true")] }), subject: "bin" });
  const ir = emit(workspace({ outputs: [tools, bin, ok] }));
  assert.equal(ir.workspace.outputs.length, 3);
});

Deno.test("a provider-backed package needs no synthetic recipe", () => {
  const jq = pkg("jq", {
    producer: provider(githubRelease({ repo: "jqlang/jq", asset: "jq-{version}.tar.gz" })),
    commands: { jq: "bin/jq" },
    target: linux,
    layout: "relocatable",
  });
  const ir = emit(workspace({ outputs: [jq] }));
  const producer = ir.workspace.outputs[0].producer;
  assert.equal(producer.kind, "provider");
  assert.equal(producer.provider.fetch.kind, "github_release");
  assert.equal(producer.provider.fetch.repo, "jqlang/jq");
  assert.ok(producer.provider.span.line >= 1, "provider fetch provenance");
  assert.match(producer.provider.span.file, /workspace\.test\.ts$/);
});

Deno.test("no import-order registry: repeated evals in one process emit identical IR", () => {
  function build(): WorkspaceValue {
    const tools = recipe("tools", {
      source: tarball("https://example.invalid/t.tar.gz"),
      execution: "native",
      output_kind: "tree",
      target: linux,
    });
    const bin = pkg("bin", {
      producer: "tools",
      commands: { t: "bin/t" },
      target: linux,
      layout: "relocatable",
    });
    return workspace({ outputs: [tools, bin] });
  }
  const first = emitWorkspaceIr(build(), facts);
  // interleave unrelated constructions and a second, different workspace
  kitchenSink();
  image("unrelated", { packages: [], target: linux });
  const second = emitWorkspaceIr(build(), facts);
  assert.equal(second, first, "same declarations, same bytes — no registry state");
});

Deno.test("constructed values are deeply frozen", () => {
  const out = recipe("tools", {
    source: tarball("https://example.invalid/t.tar.gz"),
    execution: "native",
    output_kind: "tree",
    target: linux,
    steps: [exec({ argv: [lit("make")] })],
  });
  assert.ok(Object.isFrozen(out));
  assert.ok(Object.isFrozen(out.ir));
  assert.ok(Object.isFrozen(out.ir.source));
  assert.ok(Object.isFrozen(out.ir.source.fetch));
  assert.ok(Object.isFrozen(out.ir.steps));
  assert.ok(Object.isFrozen(out.ir.steps![0]));
  assert.ok(Object.isFrozen((out.ir.steps![0] as { argv: unknown[] }).argv));
  assert.throws(() => {
    (out.ir as { name: string }).name = "other";
  }, TypeError);
  assert.throws(() => {
    ((out.ir.steps![0] as { argv: unknown[] }).argv as unknown[]).push(lit("x"));
  }, TypeError);

  const ws = kitchenSink();
  assert.ok(Object.isFrozen(ws));
  assert.ok(Object.isFrozen(ws.ir.outputs));
  assert.throws(() => {
    (ws.ir.outputs as unknown[]).push(out.ir);
  }, TypeError);
});

Deno.test("stray objects and empty catalogs are rejected", () => {
  const tools = recipe("tools", {
    source: tarball("https://example.invalid/t.tar.gz"),
    execution: "native",
    output_kind: "tree",
    target: linux,
  });
  assert.throws(
    () => workspace({ outputs: [tools, {} as never] }),
    /outputs entries must be recipe\(\)\/pkg\(\).* values — got "object"/,
  );
  assert.throws(
    () => workspace({ outputs: [false, null, undefined] }),
    /outputs must declare at least one output/,
  );
  assert.throws(
    () => emitWorkspaceIr({ __gripsack: "nope" } as never, facts),
    /emitWorkspaceIr expects a workspace\(\{\.\.\.\}\) value/,
  );
});

Deno.test("spans are mandatory: hand-built nodes without one are rejected", () => {
  const node = {
    name: "x",
    kind: "image",
    packages: [],
    target: { os: "linux", arch: "x86_64" },
  };
  const fake = {
    __gripsack: "workspace",
    ir: { span: { file: "fake.ts", line: 1 }, outputs: [node] },
  } as unknown as WorkspaceValue;
  assert.throws(() => emitWorkspaceIr(fake, facts), /span/);

  // explicit span override works for factory wrappers
  const out = image("x", {
    packages: [],
    target: linux,
    span: { file: "factory.ts", line: 7 },
  });
  const ir = emit(workspace({ outputs: [out], span: { file: "factory.ts", line: 3 } }));
  assert.equal(ir.workspace.span.file, "factory.ts");
  assert.equal(ir.workspace.outputs[0].span.line, 7);
});

Deno.test("enum and calendar boundaries reject out-of-grammar values", () => {
  assert.throws(() => daily("25:00"), /local "HH:MM"/);
  assert.throws(() => weekly("funday" as never, "10:00"), /weekday/);
  assert.throws(
    () => targetPlatform({ os: "windows" } as never),
    /os must be "linux" or "macos"/,
  );
  assert.throws(
    () =>
      recipe("x", {
        source: tarball("https://example.invalid/x.tar.gz"),
        execution: "quantum" as never,
        output_kind: "tree",
        target: linux,
      }),
    /execution must be "native", "host" or "isolated_linux"/,
  );
  // isolated_linux is admitted explicitly — the core owns the
  // unavailable-capability rejection, the frontend never reinterprets
  const iso = recipe("iso", {
    source: tarball("https://example.invalid/x.tar.gz"),
    execution: "isolated_linux",
    output_kind: "tree",
    target: linux,
  });
  assert.equal(emit(workspace({ outputs: [iso] })).workspace.outputs[0].execution, "isolated_linux");
});
