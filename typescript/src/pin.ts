/** The deliberate-pin shim (0013 D2/D3) — the deno.json import-map
 *  target for bare `@gripsack/core` imports.
 *
 * The sandboxed driver (`deno run --no-remote --cached-only --no-lock
 * --allow-read=…`) resolves the repo host entrypoint's `import … from
 * "@gripsack/core"` through the import map to THIS file, which then
 * applies the pin rule exactly once for the whole eval: the repo's own
 * `node_modules/@gripsack/core` install wins when it shadows this
 * embedded copy; the package next to this file is the fallback. The
 * driver (cli.ts) imports the same instance, so module values,
 * resource declarations, and probes always share one registry no
 * matter which copy won.
 *
 * The re-export list mirrors index.ts exactly — kept honest by the
 * pin parity test. */

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import type * as Index from "./index.ts";

/** The winning @gripsack/core URL, resolved from the repo's
 *  perspective — `repo` is the driver's first argument. */
export function resolveCoreUrl(repo: string, self: string): string {
  // the pin lives at the repo root, NOWHERE else: the eval sandbox
  // allows reading <repo> only, so a hoisted parent node_modules is
  // both unreadable and unimportable — and createRequire().resolve is
  // ambient-scope-sensitive under deno besides (it can see the
  // frontend's own package from a dev-checkout cwd)
  const pkgDir = join(resolve(repo), "node_modules", "@gripsack", "core");
  const pkgFile = join(pkgDir, "package.json");
  if (existsSync(pkgFile)) {
    const pkg = JSON.parse(readFileSync(pkgFile, "utf8")) as {
      main?: string;
      exports?: Record<string, unknown>;
    };
    const dot = pkg.exports?.["."];
    const entry = typeof dot === "string"
      ? dot
      : (dot as { import?: unknown; default?: unknown } | undefined);
    const rel =
      (typeof entry === "string" ? entry : undefined) ??
      (entry && typeof entry === "object"
        ? ((entry.import ?? entry.default) as string | undefined)
        : undefined) ??
      pkg.main ??
      "index.js";
    return pathToFileURL(join(pkgDir, rel)).href;
  }
  // the sibling entry of THIS file: index.ts in the embedded tree,
  // index.js in a tsc-built dist (string literals are not rewritten
  // by rewriteRelativeImportExtensions — only import specifiers are)
  return new URL(`./index${self.slice(-3)}`, self).href;
}

export const coreUrl = resolveCoreUrl(resolve(process.argv[2] ?? "."), import.meta.url);

// plugin loading: which package instance wins is decided at runtime
// (the repo's pin vs this embedded copy) — a static import would
// always load the embedded one and defeat the pin rule
let api: typeof Index;
try {
  api = await import(coreUrl) as typeof Index;
} catch (e) {
  console.error(
    `gripsack: the pinned @gripsack/core at ${coreUrl} failed to load: ` +
      `${(e as Error).message} — fix or remove the pin (node_modules/@gripsack/core)`,
  );
  process.exit(1);
}

/** The winning package instance — the driver and every host module
 *  share it. */
export const core = api;

export const dep = api.dep;
export const merge = api.merge;
export const symlink = api.symlink;
export const template = api.template;
export const trackedCopy = api.trackedCopy;
export const brew = api.brew;
export const fileFetch = api.fileFetch;
export const git = api.git;
export const githubRelease = api.githubRelease;
export const pixi = api.pixi;
export const pluginFetch = api.pluginFetch;
export const tarball = api.tarball;
export const hasTag = api.hasTag;
export const when = api.when;
export const defineEnv = api.defineEnv;
export const emitIr = api.emitIr;
export const IR_VERSION = api.IR_VERSION;
export const mergeTags = api.mergeTags;
export const tree = api.tree;
export const module = api.module;
export const customHook = api.customHook;
export const desktopEntry = api.desktopEntry;
export const fonts = api.fonts;
export const service = api.service;
export const parseInputs = api.parseInputs;
export const createProbeBuilder = api.createProbeBuilder;
export const CORE_RESOURCES = api.CORE_RESOURCES;
export const clearResources = api.clearResources;
export const resource = api.resource;
export const buildStep = api.buildStep;
export const configStep = api.configStep;
export const fetchStep = api.fetchStep;
export const installStep = api.installStep;
export const runStep = api.runStep;
export const shellStep = api.shellStep;
export const step = api.step;
export const verifyBinary = api.verifyBinary;
export const verifyDeployed = api.verifyDeployed;
export const verifyFile = api.verifyFile;
export const verifyShell = api.verifyShell;

export type {
  Condition,
  Dependency,
  Dest,
  Edge,
  Env,
  EnvContext,
  EnvFn,
  FactView,
  Fetch,
  HostFacts,
  Intent,
  Inputs,
  IrEntry,
  IrModule,
  ModuleSpec,
  ModuleValue,
  Ownership,
  Phase,
  ProbeBuilder,
  ProbeKind,
  ProbeRequest,
  Resource,
  Span,
  Step,
  StepAction,
  StepOpts,
  Trigger,
  Verify,
} from "./index.ts";
