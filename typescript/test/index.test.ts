/** Pure facts, injected inputs, probe and workspace provenance contracts. */
import assert from "node:assert/strict";
import { hasTag, when, profile, workspace } from "../src/index.ts";
import type { WorkspaceContext, HostFacts } from "../src/index.ts";
import { emitWorkspaceIr, parseInputs, createProbeBuilder } from "../src/advanced.ts";
import type { ProbeRequest } from "../src/advanced.ts";

const facts: HostFacts = { os: "linux", arch: "x86_64", libc: "glibc-2.36", hostname: "box" };

function view(over: Partial<WorkspaceContext> = {}): WorkspaceContext {
  const { probe } = createProbeBuilder({});
  return { facts, tags: [], probe, settings: {}, ...over };
}

Deno.test("workspace declarations capture source spans", () => {
  const ir = JSON.parse(emitWorkspaceIr(workspace({ outputs: [profile("dotfiles", {})] }), facts, []));
  assert.equal(ir.ir_version, 9);
  assert.deepEqual(Object.keys(ir), ["ir_version", "host", "workspace"]);
  assert.match(ir.workspace.outputs[0].span.file, /index\.test\.ts$/);
  assert.ok(ir.workspace.outputs[0].span.line > 0);
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

