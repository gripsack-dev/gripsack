import assert from "node:assert/strict";
import { exec, lit, profile, service, task, workspace } from "../src/index.ts";
import type { ServiceSpec, WorkspaceOutput } from "../src/index.ts";
import { emitWorkspaceIr } from "../src/advanced.ts";

const facts = { os: "linux", arch: "x86_64", libc: null, hostname: "test" };
const spec: ServiceSpec = {
  image: `docker.io/library/postgres@sha256:${"a".repeat(64)}`,
  ports: [{ address: "127.0.0.1", host: 5432, container: 5432 }],
  volumes: [{ name: "postgres-data", target: "/var/lib/postgresql/data" }],
  env: { POSTGRES_PASSWORD: { kind: "inherited", name: "POSTGRES_PASSWORD" } },
  health: { kind: "command", argv: ["pg_isready"], interval_seconds: 1, timeout_seconds: 5, retries: 10 },
};
const emit = (...outputs: WorkspaceOutput[]) => JSON.parse(emitWorkspaceIr(workspace({ outputs }), facts));

Deno.test("services emit persistent declarations and typed task readiness prerequisites", () => {
  const db = service("db", spec);
  const migrate = task("migrate", { steps: [exec({ argv: [lit("true")] })], services: ["db"] });
  const ir = emit(db, migrate);
  assert.equal(ir.ir_version, 10);
  assert.deepEqual(ir.workspace.outputs[0].command, []);
  assert.deepEqual(ir.workspace.outputs[0].depends_on, []);
  assert.deepEqual(ir.workspace.outputs[1].services, ["db"]);
  assert.deepEqual(ir.workspace.outputs[0].env.POSTGRES_PASSWORD, { kind: "inherited", name: "POSTGRES_PASSWORD" });
  assert.ok(Object.isFrozen(db.ir.health.argv));
});

Deno.test("service dependencies reject unknown outputs, wrong kinds and cycles", () => {
  assert.throws(() => emit(service("db", { ...spec, depends_on: ["missing"] })));
  assert.throws(() => emit(service("db", { ...spec, depends_on: ["files"] }), profile("files", {})));
  assert.throws(() => emit(service("a", { ...spec, depends_on: ["b"] }), service("b", { ...spec, depends_on: ["a"] })));
  assert.throws(() => emit(task("work", { steps: [exec({ argv: [lit("true")] })], services: ["missing"] })));
});

Deno.test("service declarations reject ambient networking, host volumes and unbounded health", () => {
  for (const change of [
    { image: "postgres:latest" },
    { image: `${"a".repeat(256)}@sha256:${"a".repeat(64)}` },
    { image: `postgres/@sha256:${"a".repeat(64)}` },
    { image: `postgres:@sha256:${"a".repeat(64)}` },
    { ports: [{ address: "0.0.0.0", host: 5432, container: 5432 }] },
    { ports: [{ address: "127.00.0.1", host: 5432, container: 5432 }] },
    { ports: [{ address: "::1", host: 0, container: 5432 }] },
    { ports: [...spec.ports!, ...spec.ports!] },
    { volumes: [{ name: "/tmp/data", target: "/data" }] },
    { volumes: [{ name: "a".repeat(256), target: "/data" }] },
    { volumes: [{ name: "data", target: "/" }] },
    { volumes: [...spec.volumes!, ...spec.volumes!] },
    { health: { ...spec.health, retries: 101 } },
    { health: { ...spec.health, timeout_seconds: 3601 } },
    { health: { ...spec.health, interval_seconds: 0 } },
    { health: { ...spec.health, argv: [] } },
    { command: ["x".repeat(65537)] },
    { command: Array(1025).fill("x") },
    { env: { BAD: { kind: "literal", value: "\0" } } },
    { depends_on: Array(129).fill("db") },
    { privileged: true },
    { restart: "always" },
  ]) {
    assert.throws(() => service("invalid", { ...spec, ...change } as ServiceSpec));
  }
  assert.doesNotThrow(() => service("ipv6", { ...spec, ports: [{ address: "::1", host: 5432, container: 5432 }] }));
});
