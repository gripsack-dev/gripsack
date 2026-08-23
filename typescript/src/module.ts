/** Module declaration and span capture (0004 §2). */

import type { Dependency } from "./deps.js";
import type { Dest, Ownership } from "./entries.js";
import type { Fetch } from "./fetch.js";
import type { Intent } from "./intents.js";
import type { Step } from "./steps.js";
import type { Verify } from "./verify.js";
import { register, registerClass } from "./graph.js";

export interface Span {
  file: string;
  line: number;
  col?: number;
}

export interface ModuleSpec {
  /** Optional for dotfiles-only modules (0006 §2 level 1). Mutually
   *  exclusive with `steps` (0007 §1). */
  fetch?: Fetch;
  build?: { kind: "none" | "cargo_install" | "make" } | { kind: "custom_shell"; script: string };
  install?: Record<string, Dest>;
  config?: Record<string, Dest>;
  depends?: Dependency[];
  activate?: Intent[];
  /** Explicit pipeline control (0007). */
  steps?: Step[];
  /** Module-level smoke contract, run pre-flip (0007 §verify). */
  verify?: Verify;
  /** Retry default for this module's steps (0007 §retries). */
  retries?: number;
}

export interface IrEntry {
  from: string;
  to: string;
  mode: Ownership;
}

export interface IrModule {
  fetch?: Fetch;
  build?: ModuleSpec["build"];
  install?: IrEntry[];
  config?: IrEntry[];
  depends?: Dependency[];
  activate?: Intent[];
  steps?: Step[];
  verify?: Verify;
  retries?: number;
  span?: Span;
}

/** Pipeline order for the class style (0007 §verify). */
const PIPELINE_PHASES = ["fetch", "build", "install", "config", "verify", "activate"] as const;

type PhaseName = (typeof PIPELINE_PHASES)[number];
type StepsResult = Step | Step[] | void;

/**
 * Base class for the class authoring style (0007 §1).
 *
 * Subclass and override any phase method — each returns a step, a list
 * of steps, or nothing. The pipeline chains phases in order and
 * sequences steps within each phase, filling only *empty* `needs` —
 * explicit `needs` always win. Register with {@link define}.
 *
 * **Phase methods run at eval time only** — they build IR, they never
 * run while your system is being built.
 *
 * @example
 * ```ts
 * class Helix extends Module {
 *   override fetch() {
 *     return fetchStep(githubRelease({ repo: "helix-editor/helix", asset: "h.tar.xz" }));
 *   }
 *   override install() {
 *     return installStep({ "bin/hx": symlink("~/.local/bin/hx") });
 *   }
 * }
 * define(Helix);
 * ```
 */
export abstract class Module {
  /** Module name; defaults to the class name lowercased. */
  static moduleName?: string;

  fetch(): StepsResult {
    return;
  }
  build(): StepsResult {
    return;
  }
  install(): StepsResult {
    return;
  }
  config(): StepsResult {
    return;
  }
  /** Smoke contract, run pre-flip — return a `Verify` (same object the
   *  data style's `verify` field takes) or verify steps. */
  verify(): StepsResult | Verify {
    return;
  }
  activate(): StepsResult {
    return;
  }
}

function normalize(result: StepsResult, phase: PhaseName): Step[] {
  if (!result) return [];
  const steps = Array.isArray(result) ? result : [result];
  return steps.map((s) => (s.phase === undefined ? { ...s, phase } : s));
}

/** @internal Gather phase methods into a chained step list, plus the
 *  module-level verify contract if `verify()` returned one. */
export function collectPipeline(instance: Module): { steps: Step[]; verify?: Verify } {
  const chained: Step[] = [];
  let verify: Verify | undefined;
  for (const phase of PIPELINE_PHASES) {
    const result = instance[phase]();
    if (phase === "verify" && result && !Array.isArray(result) && "kind" in result) {
      verify = result as Verify;
      continue;
    }
    for (const s of normalize(result as StepsResult, phase)) {
      const prev = chained[chained.length - 1];
      chained.push(
        s.needs === undefined && prev ? { ...s, needs: [prev.id] } : s,
      );
    }
  }
  return verify === undefined ? { steps: chained } : { steps: chained, verify };
}

/** Capture the first stack frame outside the package (V8:
 *  file:line:col) — a helper wrapper in user code must never steal
 *  the span. */
function callerSpan(): Span | undefined {
  const pkgDir = new URL(".", import.meta.url).pathname;
  const stack = new Error().stack;
  if (!stack) return undefined;
  for (const line of stack.split("\n").slice(1)) {
    const m = line.match(/\(?([^()\s]+):(\d+):(\d+)\)?$/);
    if (m && m[1] && !m[1].replace(/^file:\/\//, "").startsWith(pkgDir)) {
      const file = m[1].replace(/^file:\/\//, "");
      return { file, line: Number(m[2]), col: Number(m[3]) };
    }
  }
  return undefined;
}

/** Declare a module from declarative fields (data style). */
export function module(name: string, spec: ModuleSpec): void {
  const ir: IrModule = {};
  if (spec.fetch) ir.fetch = spec.fetch;
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
  if (spec.verify) ir.verify = spec.verify;
  if (spec.retries !== undefined) ir.retries = spec.retries;
  const span = callerSpan();
  if (span) ir.span = span;
  register(name, ir);
}

/** Register a class-style module (see {@link Module}). Instantiation is
 *  deferred to emit time — defining a module never *does* anything. */
export function define(ctor: new () => Module): void {
  const named = ctor as unknown as { moduleName?: string };
  registerClass(named.moduleName ?? ctor.name.toLowerCase(), ctor, callerSpan());
}
