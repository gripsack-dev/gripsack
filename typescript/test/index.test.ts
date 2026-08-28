/** Frontend contract tests (plan/0003 §5, 0013): emit shape, spans,
 * inputs envelope, probes, defineEnv. Runs under `deno test` (the
 * sandbox spawn itself is exercised in driver.test.ts). */

import assert from "node:assert/strict";
import {
  clearResources,
  configStep,
  define,
  dep,
  emitIr,
  fetchStep,
  githubRelease,
  hasTag,
  installStep,
  mergeTags,
  Module,
  module,
  parseInputs,
  resource,
  runStep,
  service,
  shellStep,
  symlink,
  tarball,
  trackedCopy,
  when,
} from "../src/index.ts";
import type { Env, EnvContext, HostFacts } from "../src/index.ts";
import * as index from "../src/index.ts";
import * as pin from "../src/pin.ts";
import type { ProbeRequest } from "../src/index.ts";
import { createProbeBuilder } from "../src/probe.ts";

const facts: HostFacts = { os: "linux", arch: "x86_64", libc: "glibc-2.36", hostname: "box" };

function view(over: Partial<EnvContext> = {}): EnvContext {
  const { probe } = createProbeBuilder({});
  return { facts, tags: [], probe, settings: {}, ...over };
}

// JSON.parse's inferred `any` is deliberate here: these tests assert
// the wire shape of the emitted IR, field by field
const emit = (env: Env, tags: string[] = []) => JSON.parse(emitIr(env, facts, tags));
Deno.test("emitIr emits the IR v1 shape", () => {
  clearResources();
  const helix = module("helix", {
    fetch: githubRelease({
      repo: "helix-editor/helix",
      asset: "helix-{version}-x86_64-linux.tar.xz",
    }),
    install: { "bin/hx": symlink("~/.local/bin/hx") },
    config: { "config.toml": trackedCopy("~/.config/helix/config.toml") },
    depends: [dep("git")],
    activate: [service("syncthing")],
  });
  const git = module("git", { fetch: tarball("https://example.invalid/git.tar.xz") });

  const ir = emit({ modules: [helix, git] }, ["gui"]);

  assert.equal(ir.ir_version, 1);
  assert.deepEqual(ir.host.tags, ["gui"]);

  assert.equal(ir.modules.helix.fetch.kind, "github_release");
  assert.equal(ir.modules.helix.fetch.repo, "helix-editor/helix");
  assert.deepEqual(ir.modules.helix.install, [
    { from: "bin/hx", to: "~/.local/bin/hx", mode: "owned" },
  ]);
  assert.equal(ir.modules.helix.config[0].mode, "tracked_copy");
  assert.deepEqual(ir.modules.helix.depends, [{ module: "git", edge: "runtime" }]);
  assert.equal(ir.modules.helix.activate[0].kind, "service");
  assert.equal(ir.modules.helix.activate[0].trigger, "post_activate");

  // optional sections absent, not null — the IR is sparse by convention
  assert.equal(ir.modules.git.install, undefined);
  assert.equal(ir.modules.git.depends, undefined);
});

Deno.test("host facts keep their key order and never leak the hostname", () => {
  clearResources();
  const x = module("x", { fetch: tarball("https://example.invalid/x.tar.xz") });
  const ir = emit({ modules: [x] });
  // key order is part of the golden-corpus contract
  assert.deepEqual(Object.keys(ir.host), ["os", "arch", "tags", "libc"]);
  assert.equal("hostname" in ir.host, false);
});

Deno.test("undetectable libc is omitted from the IR host", () => {
  clearResources();
  const x = module("x", { fetch: tarball("https://example.invalid/x.tar.xz") });
  const ir = JSON.parse(emitIr({ modules: [x] }, { ...facts, libc: null }, []));
  assert.deepEqual(Object.keys(ir.host), ["os", "arch", "tags"]);
});

Deno.test("module() captures spans pointing at this file", () => {
  clearResources();
  const x = module("x", { fetch: tarball("https://example.invalid/x.tar.xz") });
  const span = emit({ modules: [x] }).modules.x.span;
  assert.ok(span, "span present");
  assert.match(span.file, /index\.test\.ts$/);
  assert.ok(span.line > 0);
});

Deno.test("falsy module entries drop out of the environment", () => {
  clearResources();
  const steam = module("steam", { fetch: tarball("https://example.invalid/s.tar.xz") });
  const git = module("git", { fetch: tarball("https://example.invalid/g.tar.xz") });
  const ir = emit({ modules: [facts.os === "plan9" && steam, git] });
  assert.equal(ir.modules.steam, undefined);
  assert.ok(ir.modules.git);
});

Deno.test("dotfiles-only modules emit no source", () => {
  clearResources();
  const helix = module("helix", {
    config: { "config.toml": trackedCopy("~/.config/helix/config.toml") },
  });
  const ir = emit({ modules: [helix] });
  assert.equal(ir.modules.helix.fetch, undefined);
  assert.equal(ir.modules.helix.config[0].mode, "tracked_copy");
});

Deno.test("explicit steps emit verbatim", () => {
  clearResources();
  const patched = module("helix-patched", {
    steps: [
      fetchStep(tarball("https://example.invalid/helix.tar.xz")),
      shellStep("patch -p1 < fix.patch", "patch", { needs: ["fetch"] }),
    ],
  });
  const steps = emit({ modules: [patched] }).modules["helix-patched"].steps;
  assert.equal(steps[0].action.kind, "fetch");
  assert.equal(steps[0].phase, "fetch");
  assert.equal(steps[1].id, "patch");
  assert.equal(steps[1].action.kind, "custom_shell");
  assert.deepEqual(steps[1].needs, ["fetch"]);
});

Deno.test("runStep emits structured argv actions with outputs", () => {
  clearResources();
  const built = module("built", {
    steps: [
      runStep(["make", "install"], "make-install", {
        needs: ["fetch"],
        outputs: ["bin/hx"],
      }),
    ],
  });
  const action = emit({ modules: [built] }).modules.built.steps[0].action;
  assert.equal(action.kind, "run");
  assert.deepEqual(action.argv, ["make", "install"]);
  assert.deepEqual(action.outputs, ["bin/hx"]);
});

Deno.test("class modules chain the pipeline", () => {
  clearResources();
  class Helix extends Module {
    override fetch() {
      return fetchStep(tarball("https://example.invalid/h.tar.xz"));
    }
    override install() {
      return installStep({ "bin/hx": symlink("~/.local/bin/hx") });
    }
    override config() {
      return configStep({ "config.toml": trackedCopy("~/.config/helix/config.toml") });
    }
  }
  const value = define(Helix);
  assert.equal(value.name, "helix");
  const helix = emit({ modules: [value] }).modules.helix;
  assert.deepEqual(
    helix.steps.map((s: { id: string }) => s.id),
    ["fetch", "install", "config"],
  );
  assert.equal(helix.steps[0].needs, undefined);
  assert.deepEqual(helix.steps[1].needs, ["fetch"]);
  assert.deepEqual(helix.steps[2].needs, ["install"]);
  assert.equal(helix.steps[2].phase, "config");
  assert.match(helix.span.file, /index\.test\.ts$/);
});

Deno.test("build-only deps keep their edge", () => {
  clearResources();
  const src = module("helix-src", {
    fetch: tarball("https://example.invalid/helix.tar.xz"),
    depends: [dep("rust", "build")],
  });
  assert.equal(emit({ modules: [src] }).modules["helix-src"].depends[0].edge, "build");
});

Deno.test("duplicate module names throw at emit with both sites", () => {
  clearResources();
  const a = module("dup", { fetch: tarball("https://example.invalid/a.tar.xz") });
  const b = module("dup", { fetch: tarball("https://example.invalid/b.tar.xz") });
  assert.throws(() => emit({ modules: [a, b] }), /duplicate module 'dup'/);
});

Deno.test("stray objects in modules throw with what they are", () => {
  clearResources();
  assert.throws(
    () => emit({ modules: [{ name: "fake" } as never] }),
    /must be module\(\)\/define\(\) values/,
  );
  assert.throws(
    () => emit({ modules: [["nope"] as never] }),
    /got "an array"/,
  );
});

Deno.test("declared resources emit and validate", () => {
  clearResources();
  resource("company.lock");
  const tool = module("tool", {
    steps: [shellStep("./sync.sh", "sync", { resources: ["company.lock"] })],
  });
  const ir = emit({ modules: [tool] });
  assert.deepEqual(ir.resources, [{ name: "company.lock" }]);
  assert.deepEqual(ir.modules.tool.steps[0].resources, ["company.lock"]);
  clearResources();
});

Deno.test("undeclared resources throw at eval", () => {
  clearResources();
  assert.throws(
    () => shellStep("true", "x", { resources: ["cargo.lokc"] }),
    /unknown resource 'cargo\.lokc'/,
  );
  clearResources();
});

Deno.test("builtin resources pass eval", () => {
  clearResources();
  const s = shellStep("cargo install hx", "build", { resources: ["cargo-lock"] });
  assert.deepEqual(s.resources, ["cargo-lock"]);
  clearResources();
});

Deno.test("mergeTags unions entrypoint and CLI tags in order", () => {
  assert.deepEqual(mergeTags(["gui", "work"], ["work", "cli"]), ["gui", "work", "cli"]);
  assert.deepEqual(mergeTags(undefined, ["cli"]), ["cli"]);
  assert.deepEqual(mergeTags(["a"], []), ["a"]);
});

Deno.test("when/hasTag evaluate the injected view", () => {
  const c = view({ tags: ["gui"] });
  assert.equal(when({ os: "linux" }, c), true);
  assert.equal(when({ os: "darwin" }, c), false);
  assert.equal(when({ libc: "glibc-2.36" }, c), true);
  assert.equal(when({ tags: ["gui"] }, c), true);
  assert.equal(when({ notTags: ["gui"] }, c), false);
  assert.equal(when({ os: "linux", notTags: ["server"] }, c), true);
  assert.equal(hasTag("gui", c), true);
  assert.equal(hasTag("server", c), false);
  // undetectable libc matches null
  const bare = view({ facts: { ...facts, libc: null } });
  assert.equal(when({ libc: null }, bare), true);
  assert.equal(when({ libc: "musl" }, bare), false);
});

Deno.test("parseInputs accepts the version 1 envelope", () => {
  const inputs = parseInputs(
    JSON.stringify({
      version: 1,
      host: "laptop",
      facts: { os: "linux", arch: "x86_64", libc: "glibc-2.36", hostname: "box" },
      tags: ["gui"],
      probes: { "executable:nvidia-smi": true },
      settings: {},
    }),
  );
  assert.equal(inputs.host, "laptop");
  assert.equal(inputs.facts.os, "linux");
  assert.equal(inputs.facts.libc, "glibc-2.36");
  assert.equal(inputs.facts.hostname, "box");
  assert.deepEqual(inputs.tags, ["gui"]);
  assert.deepEqual(inputs.probes, { "executable:nvidia-smi": true });
});

Deno.test("parseInputs fills absent optionals and null libc", () => {
  const inputs = parseInputs(
    JSON.stringify({
      version: 1,
      host: "h",
      facts: { os: "linux", arch: "x86_64", hostname: "b" },
    }),
  );
  assert.equal(inputs.facts.libc, null);
  assert.deepEqual(inputs.tags, []);
  assert.deepEqual(inputs.probes, {});
  assert.deepEqual(inputs.settings, {});
});

Deno.test("parseInputs rejects malformed envelopes", () => {
  assert.throws(() => parseInputs("{"), /not valid JSON/);
  assert.throws(
    () => parseInputs('{"version":2,"host":"h","facts":{}}'),
    /unsupported inputs version 2/,
  );
  assert.throws(() => parseInputs('{"version":1,"facts":{}}'), /'host' must be a string/);
  assert.throws(() => parseInputs('{"version":1,"host":"","facts":{}}'), /'host' must not be empty/);
  assert.throws(
    () => parseInputs('{"version":1,"host":"h","facts":{"os":1,"arch":"x","hostname":"b"}}'),
    /'facts\.os' must be a string/,
  );
  assert.throws(
    () =>
      parseInputs(
        '{"version":1,"host":"h","facts":{"os":"l","arch":"x","hostname":"b"},"tags":"gui"}',
      ),
    /'tags' must be an array of strings/,
  );
  assert.throws(
    () =>
      parseInputs(
        '{"version":1,"host":"h","facts":{"os":"l","arch":"x","hostname":"b"},"probes":{"executable:x":"yes"}}',
      ),
    /probes\['executable:x'\] must be a boolean/,
  );
});

Deno.test("probes answer bound values and record unbound requests", () => {
  const { probe, requests } = createProbeBuilder({
    "executable:deno": true,
    "file_exists:/etc/hosts": false,
  });
  assert.equal(probe.executable("deno"), true);
  assert.equal(probe.file_exists("/etc/hosts"), false);
  assert.deepEqual(requests, []);

  // unbound: false now, requested with a span for the run log
  assert.equal(probe.executable("nvidia-smi"), false);
  assert.equal(probe.executable("nvidia-smi"), false); // deduped
  assert.equal(probe.file_exists("/opt/cuda"), false);
  const first: ProbeRequest = requests[0]!;
  const second: ProbeRequest = requests[1]!;
  assert.equal(requests.length, 2);
  assert.equal(first.kind, "executable");
  assert.equal(first.name, "nvidia-smi");
  assert.match(first.span!.file, /index\.test\.ts$/);
  assert.equal(second.kind, "file_exists");
  assert.equal(second.name, "/opt/cuda");
  assert.throws(() => probe.executable(""), /must not be empty/);
});

Deno.test("pin re-exports the full index surface", () => {
  for (const k of Object.keys(index)) {
    assert.ok(k in pin, `pin.ts is missing export '${k}'`);
    if (pin.coreUrl.endsWith("/src/index.ts")) {
      // fallback instance: identity must hold (drift guard)
      assert.strictEqual(
        (pin as unknown as Record<string, unknown>)[k],
        (index as unknown as Record<string, unknown>)[k],
      );
    }
  }
  assert.ok(Object.keys(index).length >= 40);
});
