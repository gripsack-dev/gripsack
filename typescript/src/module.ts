/** Module declaration and span capture (0004 §2). `module()` is a
 * pure constructor (0013 D5): it builds and returns a
 * {@link ModuleValue}; nothing is registered anywhere. The host
 * entrypoint returns the values from `defineEnv`, and the driver
 * turns that environment into IR. */

import type { Dependency } from "./deps.ts";
import type { Dest, Ownership } from "./entries.ts";
import type { Fetch } from "./fetch.ts";
import type { Intent } from "./intents.ts";
import type { Step } from "./steps.ts";
import type { Verify } from "./verify.ts";

export interface Span {
  file: string;
  line: number;
  col?: number;
}

export interface ModuleSpec {
  /** Optional for dotfiles-only modules (0006 §2 level 1). Mutually
   *  exclusive with `steps` (0007 §1). */
  fetch?: Fetch;
  build?: { kind: "none" } | { kind: "custom_shell"; script: string };
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
  /** Registered linter for this module's config payloads (0011) —
      the core drives it (0012). */
  lint?: string;
  /** Environment contributions exported to the shell profile at
      activation (0001 §3.10) — `{"VAR": "value", "PATH+": "{store}/bin"}`;
      a trailing `+` prepends to a list var. */
  env?: Record<string, string>;
}

export interface IrEntry {
  from: string;
  to: string;
  mode: Ownership;
  vars?: Record<string, string>;
  marker?: string;
}

export interface IrEnvVar {
  name: string;
  op: "set" | "prepend";
  value: string;
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
  lint?: string;
  env?: IrEnvVar[];
  span?: Span;
}

/** A constructed module — the value `module()` returns and
 * `defineEnv` environments carry. The brand distinguishes real
 * module values from stray objects in `env.modules` (a plain field,
 * deliberately: a repo's own pinned `@gripsack/core` install and the
 * embedded copy must agree on it). */
export interface ModuleValue {
  /** @internal */
  readonly __gripsack: "module";
  name: string;
  ir: IrModule;
}
/**
 * Declare a module from declarative fields (data style) and return
 * it — the host entrypoint puts the value into `defineEnv`'s
 * `modules` array; falsy entries (a condition that didn't hold)
 * simply drop out.
 *
 * For reuse, prefer a factory function over any framework — values
 * compose:
 *
 * @example
 * ```ts
 * function langServer(name: string, repo: string) {
 *   return module(name, {
 *     fetch: githubRelease({ repo, asset: `${name}-{version}.tar.gz` }),
 *     install: { [`bin/${name}`]: symlink(`~/.local/bin/${name}`) },
 *   });
 * }
 * ```
 */
export function module(name: string, spec: ModuleSpec): ModuleValue {
  const ir: IrModule = {};
  if (spec.fetch) ir.fetch = spec.fetch;
  if (spec.build) ir.build = spec.build;
  const entries = (rec?: Record<string, Dest>): IrEntry[] | undefined =>
    rec && Object.keys(rec).length > 0
      ? Object.entries(rec).map(([from, d]) => ({
          from,
          to: d.to,
          mode: d.mode,
          ...(d.vars ? { vars: d.vars } : {}),
          ...(d.marker !== undefined ? { marker: d.marker } : {}),
        }))
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
  if (spec.lint !== undefined) ir.lint = spec.lint;
  if (spec.env !== undefined) {
    ir.env = Object.entries(spec.env).map(([name, value]) =>
      name.endsWith("+")
        ? { name: name.slice(0, -1), op: "prepend" as const, value }
        : { name, op: "set" as const, value },
    );
  }
  const span = callerSpan();
  if (span) ir.span = span;
  return { __gripsack: "module", name, ir };
}

/** @internal Capture the first stack frame outside the package (V8:
 *  file:line:col) — a helper wrapper in user code must never steal
 *  the span. Shared with the probe builder (0013 D6). */
export function callerSpan(): Span | undefined {
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
