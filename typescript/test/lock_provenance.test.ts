import assert from "node:assert/strict";
import { emitWorkspaceIr } from "../src/advanced.ts";
import { exec, lit, task, workspace } from "../src/index.ts";

Deno.test("shared locks retain the earliest source tuple without changing task use spans", () => {
  const facts = { os: "linux", arch: "x86_64", libc: null, hostname: "test" };
  const step = exec({ argv: [lit("true")] });
  const usage = (name: string, span: { file: string; line: number; col?: number }) => task(name, {
    steps: [step], locks: [{ scope: "user", key: "shared", span }],
  });
  const earliest = { file: "a", line: 2, col: 2 };
  for (const [later, first] of [
    [{ file: "a.ts", line: 1 }, earliest],
    [{ file: "a", line: 10, col: 1 }, earliest],
    [{ file: "a", line: 2, col: 10 }, earliest],
    [earliest, { file: "a", line: 2 }],
  ] as const) {
    const a = usage("a", later);
    const b = usage("b", first);
    for (const outputs of [[a, b], [b, a]]) {
      const ir = JSON.parse(emitWorkspaceIr(workspace({ outputs }), facts)).workspace;
      assert.deepEqual(ir.mutation_locks, [{ scope: "user", key: "shared", span: first }]);
      assert.deepEqual(ir.outputs[0].mutation_locks, [{ scope: "user", key: "shared", span: later }]);
      assert.deepEqual(ir.outputs[1].mutation_locks, [{ scope: "user", key: "shared", span: first }]);
    }
  }
});
