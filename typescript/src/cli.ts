/** Eval driver (plan/0005 §5): `bun run dist/src/cli.js <repo> --host <name>`.
 *
 * Imports the env repo's modules and host entrypoint, then prints the
 * eval envelope on stdout: {"ir": {...}, "diagnostics": []}. The core
 * spawns this as a subprocess; it never embeds a runtime. Error
 * diagnostics exit 1 (lints are core-side since 0012).
 */

import { readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
// one graph instance, always: the driver resolves "@gripsack/core"
// FROM THE REPO's perspective — the repo's own install wins when it
// shadows the provisioned copy (the user's deliberate pin), and
// NODE_PATH lands on the provisioned copy otherwise. Type-only
// imports stay static; the runtime import is plugin loading.
import { createRequire } from "node:module";
import type { emitIr as emitIrT, setTags as setTagsT } from "@gripsack/core";

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const repo = resolve(args[0] ?? ".");
  // plugin loading: the package instance is runtime-resolved from the
  // repo's perspective (see the header comment) — never a static import
  const repoRequire = createRequire(join(repo, "gripsack-eval.ts"));
  const coreUrl = repoRequire.resolve("@gripsack/core");
  const core = (await import(pathToFileURL(coreUrl).href)) as {
    setTags: typeof setTagsT;
    emitIr: typeof emitIrT;
  };
  const { setTags, emitIr } = core;
  let host: string | undefined;
  let extraTags: string[] = [];
  for (let i = 1; i < args.length; i++) {
    const next = args[i + 1];
    if (args[i] === "--host" && next !== undefined) {
      host = next;
      i++;
    }
    if (args[i] === "--tags" && next !== undefined) {
      extraTags = next.split(",").filter(Boolean);
      i++;
    }
  }

  // host entrypoint first: it declares tags
  let tags = extraTags;
  if (host) {
    const hostFile = join(repo, "hosts", `${host}.ts`);
    try {
      // plugin loading: the host file is runtime-selected by the user
      // repo — a static import cannot exist here
      const mod = await import(pathToFileURL(hostFile).href);
      tags = [...(mod.tags ?? tags)];
    } catch (e: unknown) {
      if ((e as { code?: string }).code !== "ERR_MODULE_NOT_FOUND") throw e;
    }
  }
  setTags(tags);

  const modulesDir = join(repo, "modules");
  let files: string[] = [];
  try {
    files = readdirSync(modulesDir)
      .filter((f) => f.endsWith(".ts"))
      .sort();
  } catch {
    // no modules dir — empty graph
  }
  for (const f of files) {
    // plugin loading: module files are discovered in the user repo at
    // runtime — a static import cannot exist here
    await import(pathToFileURL(join(modulesDir, f)).href);
  }

  const payload = { ir: JSON.parse(emitIr(tags)), diagnostics: [] };
  process.stdout.write(JSON.stringify(payload) + "\n");
}

main().catch((e) => {
  // frontend tracebacks are the frontend's domain — the core passes
  // stderr through untouched (0005 §4)
  console.error(e);
  process.exit(1);
});
