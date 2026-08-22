/**
 * gripsack typescript frontend — typed module DSL, emits IR.
 *
 * Modules are plain TypeScript using this package (plan/0001 §3.3,
 * 0005 §1). Evaluation collects Module objects into a graph and emits
 * the IR (JSON) the Rust core consumes. The core never executes this
 * code; it only reads the IR.
 *
 * Spans (0004 §2): `module()` captures the caller's file:line:col from
 * the V8 stack so core errors point back at the user's source.
 */

export const IR_VERSION = 1;

export interface Span {
  file: string;
  line: number;
  col?: number;
}

// ---------------------------------------------------------------- sources

export type Source =
  | { kind: "github_release"; repo: string; asset: string; version?: string; sha256?: string; baseUrl?: string }
  | { kind: "tarball"; url: string; sha256?: string }
  | { kind: "git"; url: string; rev: string }
  | { kind: "file"; path: string }
  | { kind: "plugin"; name: string; args?: Record<string, unknown> };

export function githubRelease(spec: {
  repo: string;
  asset: string;
  version?: string;
  sha256?: string;
  baseUrl?: string;
}): Source {
  return { kind: "github_release", ...spec };
}

export function tarball(url: string, sha256?: string): Source {
  return sha256 === undefined
    ? { kind: "tarball", url }
    : { kind: "tarball", url, sha256 };
}

export function git(url: string, rev: string): Source {
  return { kind: "git", url, rev };
}

export function fileSource(path: string): Source {
  return { kind: "file", path };
}

/** A sourcerer plugin transport (0002 §4). */
export function pluginSource(name: string, args?: Record<string, unknown>): Source {
  return args === undefined
    ? { kind: "plugin", name }
    : { kind: "plugin", name, args };
}

// ---------------------------------------------------------------- entries

export type Ownership = "owned" | "tracked_copy" | "merge" | "template";

export interface Dest {
  to: string;
  mode: Ownership;
}

/** Store-owned, read-only; edits go through the module. */
export const symlink = (to: string): Dest => ({ to, mode: "owned" });
/** Copied from the store; drift detected on next apply. */
export const trackedCopy = (to: string): Dest => ({ to, mode: "tracked_copy" });
/** Managed block merged into a file other tools also write. */
export const merge = (to: string): Dest => ({ to, mode: "merge" });
/** Rendered at activation from module variables. */
export const template = (to: string): Dest => ({ to, mode: "template" });

// ---------------------------------------------------------------- deps

export type Edge = "runtime" | "build";

export interface Dependency {
  module: string;
  edge: Edge;
}

/** `edge: "build"` = ephemeral, build-only (0001 §3.1). */
export const dep = (module: string, edge: Edge = "runtime"): Dependency => ({
  module,
  edge,
});

// ---------------------------------------------------------------- intents

export type Trigger = "post_link" | "post_activate" | "on_remove";

export type Intent =
  | { kind: "service"; trigger: Trigger; name: string; user: boolean }
  | { kind: "fonts"; trigger: Trigger }
  | { kind: "desktop_entry"; trigger: Trigger }
  | { kind: "custom_shell"; trigger: Trigger; script: string };

export const service = (
  name: string,
  user = true,
  trigger: Trigger = "post_activate",
): Intent => ({ kind: "service", trigger, name, user });

export const fonts = (trigger: Trigger = "post_link"): Intent => ({
  kind: "fonts",
  trigger,
});

export const desktopEntry = (trigger: Trigger = "post_link"): Intent => ({
  kind: "desktop_entry",
  trigger,
});

/** Escape hatch — flagged, shown by `plan`. */
export const customHook = (script: string, trigger: Trigger = "post_activate"): Intent => ({
  kind: "custom_shell",
  trigger,
  script,
});

// ---------------------------------------------------------------- modules

export interface ModuleSpec {
  source: Source;
  build?: { kind: "none" | "cargo_install" | "make" } | { kind: "custom_shell"; script: string };
  install?: Record<string, Dest>;
  config?: Record<string, Dest>;
  depends?: Dependency[];
  activate?: Intent[];
}

interface IrEntry {
  from: string;
  to: string;
  mode: Ownership;
}

interface IrModule {
  source: Source;
  build?: ModuleSpec["build"];
  install?: IrEntry[];
  config?: IrEntry[];
  depends?: Dependency[];
  activate?: Intent[];
  span?: Span;
}

const GRAPH = new Map<string, IrModule>();

/** Capture the first stack frame outside this file (V8: file:line:col). */
function callerSpan(): Span | undefined {
  const here = new URL(import.meta.url).pathname;
  const stack = new Error().stack;
  if (!stack) return undefined;
  for (const line of stack.split("\n").slice(1)) {
    const m = line.match(/\(?([^()\s]+):(\d+):(\d+)\)?$/);
    if (m && m[1] && !m[1].endsWith(here)) {
      const file = m[1].replace(/^file:\/\//, "");
      return { file, line: Number(m[2]), col: Number(m[3]) };
    }
  }
  return undefined;
}

/** Declare a module and register it in the graph. */
export function module(name: string, spec: ModuleSpec): void {
  const ir: IrModule = { source: spec.source };
  if (spec.build) ir.build = spec.build;
  const entries = (rec?: Record<string, Dest>): IrEntry[] | undefined =>
    rec && Object.keys(rec).length > 0
      ? Object.entries(rec).map(([from, d]) => ({ from, to: d.to, mode: d.mode }))
      : undefined;
  const install = entries(spec.install);
  if (install) ir.install = install;
  const config = entries(spec.config);
  if (config) ir.config = config;
  if (spec.depends?.length) ir.depends = spec.depends;
  if (spec.activate?.length) ir.activate = spec.activate;
  const span = callerSpan();
  if (span) ir.span = span;
  GRAPH.set(name, ir);
}

/** Drop all registered modules (test isolation). */
export function clearGraph(): void {
  GRAPH.clear();
}

export interface HostFacts {
  os: string;
  arch: string;
  tags: string[];
}

/** Serialize the registered graph as IR JSON (plan/0001 §3.2). */
export function emitIr(tags: string[] = [], host?: Partial<HostFacts>): string {
  const ir = {
    ir_version: IR_VERSION,
    host: {
      os: host?.os ?? process.platform,
      arch: host?.arch ?? process.arch,
      tags,
    },
    modules: Object.fromEntries(GRAPH),
  };
  return JSON.stringify(ir, null, 2);
}
