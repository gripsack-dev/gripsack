/** Module declaration and span capture (0004 §2). */

import type { Dependency } from "./deps.js";
import type { Dest, Ownership } from "./entries.js";
import type { Intent } from "./intents.js";
import type { Source } from "./sources.js";
import type { Step } from "./steps.js";
import { register } from "./graph.js";

export interface Span {
  file: string;
  line: number;
  col?: number;
}

export interface ModuleSpec {
  /** Optional for dotfiles-only modules (0006 §2 level 1). Mutually
   *  exclusive with `steps` (0007 §1). */
  source?: Source;
  build?: { kind: "none" | "cargo_install" | "make" } | { kind: "custom_shell"; script: string };
  install?: Record<string, Dest>;
  config?: Record<string, Dest>;
  depends?: Dependency[];
  activate?: Intent[];
  /** Explicit pipeline control (0007). */
  steps?: Step[];
}

export interface IrEntry {
  from: string;
  to: string;
  mode: Ownership;
}

export interface IrModule {
  source?: Source;
  build?: ModuleSpec["build"];
  install?: IrEntry[];
  config?: IrEntry[];
  depends?: Dependency[];
  activate?: Intent[];
  steps?: Step[];
  span?: Span;
}

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
  const ir: IrModule = {};
  if (spec.source) ir.source = spec.source;
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
  if (spec.steps?.length) ir.steps = spec.steps;
  const span = callerSpan();
  if (span) ir.span = span;
  register(name, ir);
}
