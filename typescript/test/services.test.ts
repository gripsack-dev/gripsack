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

Deno.test("task prerequisites and service dependencies enforce the same IR10 name limits", () => {
  const limit = "é".repeat(32768);
  for (const declare of [
    (names: string[]) => service("consumer", { ...spec, depends_on: names }).ir.depends_on,
    (names: string[]) => task("consumer", { steps: [exec({ argv: [lit("true")] })], services: names }).ir.services,
  ]) {
    const names = [limit, ...Array.from({ length: 127 }, (_, i) => `db-${i}`)];
    assert.deepEqual(declare(names), names);
    for (const invalid of [[""], ["db", "db"], ["db\n"], ["db\u0085"], [limit + "x"], [...names, "extra"]]) {
      assert.throws(() => declare(invalid));
    }
  }
});

Deno.test("optional image command and required health command bound the executable and each argument", () => {
  const limit = "é".repeat(32768);
  assert.deepEqual(service("db", { ...spec, command: [] }).ir.command, []);
  assert.throws(() => service("db", { ...spec, health: { ...spec.health, argv: [] } }));
  for (const declare of [
    (argv: string[]) => service("db", { ...spec, command: argv }).ir.command,
    (argv: string[]) => service("db", { ...spec, health: { ...spec.health, argv } }).ir.health.argv,
  ]) {
    const valid = ["run", limit, ...Array<string>(1022).fill("")];
    assert.deepEqual(declare(valid), valid);
    for (const invalid of [[""], ["", "x"], ["run", "\0"], ["run", limit + "x"], [...valid, "x"]]) {
      assert.throws(() => declare(invalid));
    }
  }
});

Deno.test("service environment keys and inherited names are bounded ASCII identifiers", () => {
  const limit = "A".repeat(65536);
  for (const env of [
    (name: string) => ({ [name]: { kind: "literal" as const, value: "" } }),
    (name: string) => ({ KEY: { kind: "inherited" as const, name } }),
  ]) {
    assert.doesNotThrow(() => service("db", { ...spec, env: env(limit) }));
    for (const name of [limit + "A", "1KEY", "KÉY", "KEY\n"]) {
      assert.throws(() => service("db", { ...spec, env: env(name) }));
    }
  }
});
