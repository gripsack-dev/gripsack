/** Driver-owned package selection. Both public and compiler APIs come from
 * the same approved package; a historical pin keeps its original root ABI. */
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import type * as Index from "./index.ts";
import type * as Advanced from "./advanced.ts";

type Entry = string | { import?: string; default?: string; types?: string };
interface PackageEntries {
  main?: string;
  types?: string;
  exports?: Record<string, Entry>;
}
function resolveEntry(directory: string, entry: Entry | undefined, fallback: (string | undefined)[]): string {
  const candidates = typeof entry === "string" ? [entry, ...fallback] : [entry?.import ?? entry?.default, ...fallback, entry?.types];
  for (const candidate of candidates) {
    if (typeof candidate === "string" && existsSync(join(directory, candidate))) {
      return pathToFileURL(join(directory, candidate)).href;
    }
  }
  throw new Error(`pinned @gripsack/core has no readable entry in ${directory}`);
}
function selectEntries(): { authoring: string; compiler: string } {
  // Deliberate pins live at this repository root, never in ambient ancestors.
  const directory = join(resolve(process.argv[2] ?? "."), "node_modules", "@gripsack", "core");
  const manifest = join(directory, "package.json");
  if (!existsSync(manifest)) {
    return { authoring: new URL("./index.ts", import.meta.url).href, compiler: new URL("./advanced.ts", import.meta.url).href };
  }
  const pkg: PackageEntries = JSON.parse(readFileSync(manifest, "utf8"));
  const authoring = resolveEntry(directory, pkg.exports?.["."], [pkg.main, pkg.types, "index.js"]);
  const advanced = pkg.exports?.["./advanced"];
  return { authoring, compiler: advanced === undefined ? authoring : resolveEntry(directory, advanced, []) };
}
const entries = selectEntries();
export const coreUrl = entries.authoring;
export const authoring = await import(entries.authoring) as typeof Index;
// The approved package path is chosen at runtime; a static import would bypass
// an intentional repository pin and bind the embedded compiler instead.
export const compiler = await import(entries.compiler) as typeof Advanced;
// Only the internal driver consumes this combined view. The bare import-map
// target exports authoring values only, just like the installed package root.
export const core = { ...authoring, ...compiler };
