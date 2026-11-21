import assert from "node:assert/strict";
import { conda, pixi, provider } from "../src/index.ts";

const source = { channels: ["conda-forge"], packages: { python: ">=3.12,<4" } };
const lock = { manifest: "project manifest", lock: "project-lock", environment: "GPU tools" };
const platform = { os: "linux", arch: "x86_64", minimum_os: { major: 6, minor: 1 } } as const;
const span = { file: "sources.ts", line: 3 };
const wheel = { name: "demo", version: "1.0", url: "https://FILES.example.test/pkg/demo.whl", sha256: "a".repeat(64) };

Deno.test("provider text is diagnosed, not trimmed or normalized", () => {
  for (const text of [" padded", "padded\u00a0", "inter\x7fnal", "inter\u0085nal"]) {
    for (const [values, diagnostic] of [
      [{ ...source, channels: [text] }, /channels\[0\]/],
      [{ ...source, packages: { python: text } }, /packages\["python"\]/],
    ] satisfies [typeof source, RegExp][]) {
      assert.throws(() => conda.environment(values), diagnostic);
      assert.throws(() => provider({ kind: "conda_environment", ...values, span }), diagnostic);
    }
    for (const field of ["manifest", "lock", "environment"] as const) {
      assert.throws(() => pixi.fromLock({ ...lock, [field]: text }), new RegExp(`\\.${field} must`));
    }
  }
  const locked = pixi.fromLock(lock);
  assert.deepEqual(locked, { kind: "pixi_lock", ...lock, span: locked.span });
  assert.match(locked.span.file, /provider_admission\.test\.ts$/);
});

Deno.test("duplicate platforms compare decoded fields; wheel pins retain their spelling", () => {
  const reordered = { minimum_os: { minor: 1, major: 6 }, arch: "x86_64", os: "linux" } as const;
  assert.throws(() => conda.environment({ ...source, platforms: [platform, reordered] }), /platforms\[1\].*duplicate/);
  const platforms = [platform, { ...platform, minimum_os: { ...platform.minimum_os, patch: 0 } }];
  const value = conda.environment({ ...source, platforms, wheels: [wheel] });
  assert.deepEqual(value, { ...source, kind: "conda_environment", platforms, wheels: [wheel], span: value.span });
  assert.match(value.span.file, /provider_admission\.test\.ts$/);
  for (const url of ["HTTPS://host/demo.whl", "https:host/demo.whl", "https://host\\demo.whl", "https://@host/demo.whl"]) {
    assert.throws(() => conda.environment({ ...source, wheels: [{ ...wheel, url }] }), /wheels\[0\]\.url/);
  }
});
