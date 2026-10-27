/** Driver end-to-end under the exact 0013 spawn contract:
 * `deno run --no-remote --cached-only --no-lock --allow-read=<repo>,
 * <inputs dir>,<frontend> src/cli.ts <repo> --inputs <path>`.
 * Exercises the workspace entrypoint and two-stage probe protocol. */

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const cli = join(pkgRoot, "src", "cli.ts");

type Run = { status: number | null; stdout: string; stderr: string };

/** Write an inputs file OUTSIDE the repo (its own allowed dir, like
 * the core does) and run the driver under the sandbox flags. */
function runDriver(repo: string, inputs: Record<string, unknown>): Run {
  const inputsDir = mkdtempSync(join(tmpdir(), "gs-inputs-"));
  const inputsPath = join(inputsDir, "inputs.json");
  writeFileSync(inputsPath, JSON.stringify(inputs));
  const res = spawnSync(
    Deno.execPath(),
    [
      "run",
      "--no-remote",
      "--cached-only",
      "--no-lock",
      `--allow-read=${repo},${inputsDir},${pkgRoot}`,
      cli,
      repo,
      "--inputs",
      inputsPath,
    ],
    { encoding: "utf8" },
  );
  rmSync(inputsDir, { recursive: true, force: true });
  return { status: res.status, stdout: res.stdout ?? "", stderr: res.stderr ?? "" };
}

function withRepo(files: Record<string, string>, fn: (repo: string) => void): void {
  const repo = mkdtempSync(join(tmpdir(), "gs-repo-"));
  try {
    for (const [rel, body] of Object.entries(files)) {
      const path = join(repo, rel);
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, body);
    }
    fn(repo);
  } finally {
    rmSync(repo, { recursive: true, force: true });
  }
}


const baseInputs = {
  version: 1,
  host: "lap",
  facts: { os: "linux", arch: "x86_64", libc: "glibc-2.36", hostname: "box" },
  tags: ["work"],
  probes: {},
  settings: {},
};


const WORKSPACE = `import { defineWorkspace, workspace, recipe, pkg, targetPlatform, githubRelease } from "@gripsack/core";

const tools = recipe("tools", {
  source: githubRelease({ repo: "example/tools", asset: "tools-{version}.tar.gz" }),
  execution: { kind: "host", access: "unconfined" },
  output_kind: "tree",
  target: targetPlatform({ os: "linux", arch: "x86_64" }),
});
const bin = pkg("tools-bin", {
  producer: "tools",
  commands: { tools: "bin/tools" },
  target: targetPlatform({ os: "linux", arch: "x86_64" }),
  layout: { kind: "relocatable" },
});

export default defineWorkspace((ctx) => workspace({
  outputs: [tools, ctx.probe.executable("ninja") && bin],
}));
`;

Deno.test("driver evaluates a root gripsack.ts workspace with no fake host", () => {
  // inputs.host stays "lap" but there is no hosts/ dir at all — the
  // workspace entry needs no hostname selection
  withRepo({ "gripsack.ts": WORKSPACE }, (repo) => {
    const r = runDriver(repo, baseInputs);
    assert.equal(r.status, 0, `driver failed:\n${r.stderr}`);
    const envelope = JSON.parse(r.stdout);
    assert.deepEqual(Object.keys(envelope.ir), ["ir_version", "host", "workspace"]);
    assert.equal(envelope.ir.ir_version, 9);
    assert.equal(envelope.ir.modules, undefined, "workspace envelope never carries modules");
    assert.equal(envelope.ir.host.os, "linux");
    assert.equal("hostname" in envelope.ir.host, false);
    assert.deepEqual(envelope.ir.host.tags, ["work"]);

    const outputs = envelope.ir.workspace.outputs;
    assert.equal(outputs.length, 1, "unbound probe gates tools-bin out");
    assert.equal(outputs[0].kind, "recipe");
    assert.equal(outputs[0].source.kind, "fetch");
    assert.equal(outputs[0].source.fetch.kind, "github_release");
    assert.match(outputs[0].span.file, /gripsack\.ts$/);
    assert.match(envelope.ir.workspace.span.file, /gripsack\.ts$/);
    assert.deepEqual(
      envelope.probe_requests.map((p: { kind: string; name: string }) => [p.kind, p.name]),
      [["executable", "ninja"]],
    );

    const bound = runDriver(repo, { ...baseInputs, probes: { "executable:ninja": true } });
    assert.equal(bound.status, 0, bound.stderr);
    const env2 = JSON.parse(bound.stdout);
    assert.deepEqual(
      env2.ir.workspace.outputs.map((o: { name: string }) => o.name),
      ["tools", "tools-bin"],
    );
    assert.deepEqual(env2.probe_requests, []);
  });
});

Deno.test("host identity never selects an alternate entrypoint", () => {
  withRepo({ "gripsack.ts": WORKSPACE, "hosts/lap.ts": 'throw new Error("must not execute");' }, (repo) => {
    const r = runDriver(repo, { ...baseInputs, probes: { "executable:ninja": true } });
    assert.equal(r.status, 0, `driver failed:\n${r.stderr}`);
    const envelope = JSON.parse(r.stdout);
    assert.ok(envelope.ir.workspace, "workspace envelope emitted");
    assert.equal(envelope.ir.modules, undefined, "never both envelopes");
  });
});

Deno.test("a gripsack.ts without a defineWorkspace default export errors clearly", () => {
  withRepo({ "gripsack.ts": "export const outputs = [];\n" }, (repo) => {
    const r = runDriver(repo, baseInputs);
    assert.equal(r.status, 1);
    assert.match(r.stderr, /must default-export defineWorkspace/);
  });
});

// ---------------------------------------------------------------------------
// A1-06 classification boundary: a syntax error in the workspace file
// is an engine failure — the import rethrows to the traceback path
// (0005 §4), never a synthetic E130 envelope.

Deno.test("a syntax error in gripsack.ts stays a traceback", () => {
  withRepo(
    {
      "gripsack.ts": 'import { defineWorkspace } from "@gripsack/core";\n' +
        "export default defineWorkspace(() => {\n",
    },
    (repo) => {
      const r = runDriver(repo, baseInputs);
      assert.equal(r.status, 1);
      assert.equal(r.stdout, "");
      assert.match(r.stderr, /gripsack\.ts/);
      assert.doesNotMatch(r.stderr, /E130/);
    },
  );
});

Deno.test("a host file never substitutes for a missing workspace", () => {
  withRepo({ "hosts/lap.ts": 'throw new Error("must not execute");' }, (repo) => {
    const result = runDriver(repo, baseInputs);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /no gripsack\.ts/);
    assert.doesNotMatch(result.stderr, /must not execute/);
  });
});

Deno.test("pins must expose the explicit compiler entrypoint", () => {
  withRepo({
    "gripsack.ts": WORKSPACE,
    "node_modules/@gripsack/core/package.json": '{"name":"@gripsack/core","type":"module","main":"index.js"}',
    "node_modules/@gripsack/core/index.js": "export const emitWorkspaceIr = () => '{}';",
  }, (repo) => {
    const result = runDriver(repo, baseInputs);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /must export \.\/advanced/);
  });
});

Deno.test("workspace authoring failures retain source-located diagnostics", () => {
  withRepo({
    "gripsack.ts": 'import { defineWorkspace, workspace, profile } from "@gripsack/core";\n' +
      'const broken = profile("broken", { filez: [] });\n' +
      'export default defineWorkspace(() => workspace({ outputs: [broken] }));\n',
  }, (repo) => {
    const result = runDriver(repo, baseInputs);
    assert.equal(result.status, 1);
    const envelope = JSON.parse(result.stdout);
    assert.equal(envelope.ir, null);
    assert.match(envelope.diagnostics[0].message, /filez/);
    assert.ok(envelope.diagnostics[0].labels.some(
      (label: { span?: { file: string; line: number } }) =>
        label.span?.file.endsWith("gripsack.ts") && label.span.line === 2,
    ));
  });
});
