import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { pkg, provider, tarball } from "../src/pin.ts";
import { release } from "./fixtures/span_helper.ts";

const here = fileURLToPath(import.meta.url);

Deno.test("a helper's declaration is located at the caller, not inside the helper", () => {
  const site = new Error().stack!; // keep `release(` on the next line
  const output = release("hunk");
  const line = Number(site.match(/span_capture\.test\.ts:(\d+)/)![1]) + 1;
  assert.equal(output.ir.span.file, here);
  assert.equal(output.ir.span.line, line);
  assert.ok(!output.ir.span.file.endsWith("span_helper.ts"));
});

Deno.test("a declaration written directly keeps its own site", () => {
  const site = new Error().stack!;
  const output = pkg("direct", {
    producer: provider(tarball("file:///tmp/x.tar.gz")),
    commands: { direct: "direct" },
    target: { os: "linux", arch: "x86_64" },
    layout: { kind: "relocatable" },
  });
  const line = Number(site.match(/span_capture\.test\.ts:(\d+)/)![1]) + 1;
  assert.equal(output.ir.span.file, here);
  assert.equal(output.ir.span.line, line);
});
