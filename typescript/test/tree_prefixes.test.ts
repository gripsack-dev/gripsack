import assert from "node:assert/strict";
import { artifactTree, file, identity, inputDirectory, repoTree, trackedCopyTo } from "../src/index.ts";

Deno.test("tree selection rejects glob syntax in either list before emission", () => {
  for (const key of ["include", "exclude"] as const) {
    for (const prefix of ["bin/*", "bin/?", "bin/[ab]", "bin/{a,b}", "bin/{unknown}", "bin/{version", "bin/file\x7f"]) {
      const filters = { include: ["bin"], [key]: [prefix] };
      for (const declare of [
        () => repoTree(".", filters),
        () => inputDirectory("assets", ".", filters),
        () => artifactTree("tool", filters),
        () => file({ source: { kind: "tree", output: "tool", ...filters }, content: identity(), destination: trackedCopyTo("~/.bin") }),
      ]) assert.throws(declare, new RegExp(`${key}.*segment prefix, not a glob`));
    }
    const tokens = "release-{version}-{version.bare}/{os}/{arch}/{arch.go}/{arch.x64}/{target}/{system}";
    const filters = { include: ["bin"], [key]: [tokens] };
    assert.deepEqual(artifactTree("tool", filters), { kind: "tree", output: "tool", ...filters });
    assert.throws(() => repoTree(".", filters), /segment prefix/);
    assert.throws(() => inputDirectory("assets", ".", filters), /segment prefix/);
  }
  const literal = { include: ["bin"], exclude: ["bin/debug"] };
  assert.deepEqual(repoTree(".", literal), { kind: "repo_tree", path: ".", ...literal });
  assert.deepEqual(inputDirectory("assets", ".", literal).origin, { kind: "repo_directory", path: ".", ...literal });
});
