/** Driver end-to-end under the exact 0013 spawn contract:
 * `deno run --no-remote --cached-only --no-lock --allow-read=<repo>,
 * <inputs dir>,<frontend> src/cli.ts <repo> --inputs <path>`.
 * Exercises host selection and the two-stage probe protocol. */

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

const HOST = `import { defineEnv, module, trackedCopy, githubRelease, symlink } from "@gripsack/core";

const helix = module("helix", {
  fetch: githubRelease({ repo: "helix-editor/helix", asset: "helix.tar.xz" }),
  install: { "bin/hx": symlink("~/.local/bin/hx") },
});
const demo = module("demo", {
  config: { "configs/demo/demo.toml": trackedCopy("~/.config/demo/demo.toml") },
});
const cuda = module("cuda", {
  fetch: githubRelease({ repo: "nvidia/cuda", asset: "cuda.tar.xz" }),
});

export default defineEnv((ctx) => ({
  tags: ["gui"],
  modules: [
    helix,
    ctx.facts.os === "linux" && demo,
    ctx.probe.executable("nvidia-smi") && cuda,
  ],
}));
`;

const baseInputs = {
  version: 1,
  host: "lap",
  facts: { os: "linux", arch: "x86_64", libc: "glibc-2.36", hostname: "box" },
  tags: ["work"],
  probes: {},
  settings: {},
};

Deno.test("driver evaluates a host into an envelope under the sandbox flags", () => {
  withRepo({ "hosts/lap.ts": HOST }, (repo) => {
    const r = runDriver(repo, baseInputs);
    assert.equal(r.status, 0, `driver failed:\n${r.stderr}`);
    const envelope = JSON.parse(r.stdout);
    assert.deepEqual(Object.keys(envelope), ["ir", "diagnostics", "probe_requests"]);
    assert.deepEqual(envelope.diagnostics, []);

    // entrypoint tags ∪ CLI tags
    assert.deepEqual(envelope.ir.host, {
      os: "linux",
      arch: "x86_64",
      tags: ["gui", "work"],
      libc: "glibc-2.36",
    });
    assert.ok(envelope.ir.modules.helix);
    assert.ok(envelope.ir.modules.demo, "facts-conditional module present");
    assert.equal(envelope.ir.modules.cuda, undefined, "unbound probe gates cuda out");
    assert.equal(envelope.ir.modules.helix.span.file.endsWith("lap.ts"), true);

    // stage 1: the unbound probe is a request, not an answer
    assert.deepEqual(
      envelope.probe_requests.map((p: { kind: string; name: string }) => [p.kind, p.name]),
      [["executable", "nvidia-smi"]],
    );
    assert.equal(envelope.probe_requests[0].span.file.endsWith("lap.ts"), true);
  });
});

Deno.test("bound probes answer without new requests (stage 2 fixpoint)", () => {
  withRepo({ "hosts/lap.ts": HOST }, (repo) => {
    const yes = runDriver(repo, { ...baseInputs, probes: { "executable:nvidia-smi": true } });
    assert.equal(yes.status, 0, yes.stderr);
    const env = JSON.parse(yes.stdout);
    assert.ok(env.ir.modules.cuda, "bound-true admits cuda");
    assert.deepEqual(env.probe_requests, []);

    const no = runDriver(repo, { ...baseInputs, probes: { "executable:nvidia-smi": false } });
    assert.equal(no.status, 0, no.stderr);
    const env2 = JSON.parse(no.stdout);
    assert.equal(env2.ir.modules.cuda, undefined, "bound-false keeps cuda out");
    assert.deepEqual(env2.probe_requests, []);
  });
});

Deno.test("a missing host file lists the available hosts", () => {
  withRepo({ "hosts/lap.ts": HOST }, (repo) => {
    const r = runDriver(repo, { ...baseInputs, host: "other" });
    assert.equal(r.status, 1);
    assert.match(r.stderr, /no hosts\/other\.ts \(have: lap\) /);
  });
});

Deno.test("a host without a defineEnv default export errors clearly", () => {
  withRepo(
    { "hosts/bad.ts": 'export const tags = ["gui"];\n' },
    (repo) => {
      const r = runDriver(repo, { ...baseInputs, host: "bad" });
      assert.equal(r.status, 1);
      assert.match(r.stderr, /must default-export defineEnv/);
    },
  );
});

Deno.test("duplicate module names surface the frontend error", () => {
  withRepo(
    {
      "hosts/dup.ts":
        'import { defineEnv, module } from "@gripsack/core";\n' +
        'const a = module("x", { lint: "toml" });\n' +
        'const b = module("x", { lint: "toml" });\n' +
        "export default defineEnv(() => ({ modules: [a, b] }));\n",
    },
    (repo) => {
      const r = runDriver(repo, { ...baseInputs, host: "dup" });
      assert.equal(r.status, 1);
      assert.match(r.stderr, /duplicate module 'x'/);
    },
  );
});


Deno.test("a stale pin predating defineEnv errors with the fix", () => {
  const STALE = `export const defineEnv = (fn) => fn;
  export const module = (name, spec) => ({ __gripsack: "module", name, ir: spec });
  `;
  withRepo(
    {
      "hosts/old.ts":
        'import { defineEnv, module } from "@gripsack/core";\n' +
        'const a = module("x", { lint: "toml" });\n' +
        "export default defineEnv(() => ({ modules: [a] }));\n",
      "node_modules/@gripsack/core/package.json":
        '{"name":"@gripsack/core","version":"0.16.4","type":"module","main":"index.js"}',
      "node_modules/@gripsack/core/index.js": STALE,
    },
    (repo) => {
      const r = runDriver(repo, { ...baseInputs, host: "old" });
      assert.equal(r.status, 1);
      assert.match(r.stderr, /predates the defineEnv frontend/);
      assert.match(r.stderr, /node_modules\/@gripsack\/core/);
    },
  );
});

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
    assert.equal(envelope.ir.ir_version, 7);
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

Deno.test("a root gripsack.ts wins over hosts/<host>.ts", () => {
  withRepo({ "gripsack.ts": WORKSPACE, "hosts/lap.ts": HOST }, (repo) => {
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

Deno.test("retired Pixi calls report a source diagnostic through an imported alias", () => {
  withRepo(
    {
      "hosts/lap.ts":
        'import { defineEnv, module, pixi as acquire } from "@gripsack/core";\n' +
        'export default defineEnv(() => ({ modules: [module("old", { fetch: acquire("nodejs") })] }));\n',
    },
    (repo) => {
      const result = runDriver(repo, baseInputs);
      assert.equal(result.status, 1);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /E130/);
      assert.match(result.stderr, /workspace-migration\.html/);
      assert.match(result.stderr, /lap\.ts:2/);
    },
  );
});
