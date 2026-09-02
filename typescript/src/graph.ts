/** The environment contract (0013 D5): `Inputs → Environment` as a
 *  function. No global registry, no import-order magic — the host
 *  entrypoint RETURNS its modules, the driver turns that value into
 *  IR (JSON). */

import type { HostFacts } from "./facts.ts";
import type { FactView } from "./conditions.ts";
import type { IrModule, ModuleValue } from "./module.ts";
import type { ProbeBuilder } from "./probe.ts";
import { declaredResources } from "./resources.ts";

export const IR_VERSION = 1;

/** The context a `defineEnv` function receives (0013 D5/D6): every
 *  host observation arrives here — facts and tags core-injected,
 *  probes symbolic, settings reserved. */
export interface EnvContext extends FactView {
  facts: HostFacts;
  tags: string[];
  probe: ProbeBuilder;
  settings: Record<string, unknown>;
}

/** A host environment: the entrypoint's tags plus its module values.
 *  Falsy `modules` entries drop out — `ctx.facts.os === "linux" &&
 *  steam` is the conditional style. */
export interface Env {
  tags?: string[];
  modules: ReadonlyArray<ModuleValue | false | null | undefined>;
}

export type EnvFn = (ctx: EnvContext) => Env;

/**
 * Declare a host entrypoint:
 *
 * ```ts
 * // hosts/laptop.ts
 * import { defineEnv } from "@gripsack/core";
 * import { helix } from "../modules/helix.js";
 *
 * export default defineEnv((ctx) => ({
 *   tags: ["gui", "work"],
 *   modules: [
 *     helix,
 *     ctx.facts.os === "linux" && steam,
 *     ctx.probe.executable("nvidia-smi") && cuda,
 *   ],
 * }));
 * ```
 *
 * The function runs inside the sandboxed eval with the core-injected
 * context and must return the environment synchronously. Identity —
 * the marker the driver checks for; the API anchor is the function
 * itself.
 */
export function defineEnv(fn: EnvFn): EnvFn {
  return fn;
}

/** Entrypoint tags ∪ CLI tags, order-preserving, deduplicated. The
 *  result is what lands in `ir.host.tags` and what a re-run's
 *  `ctx.tags` will contain (the core feeds the union back). */
export function mergeTags(envTags: string[] | undefined, cliTags: string[]): string[] {
  return [...(envTags ?? []), ...cliTags].filter((t, i, all) => all.indexOf(t) === i);
}

/** Serialize a returned environment as IR JSON. Duplicate module
 *  names throw with both declaration sites; stray objects throw with
 *  what they are. Key order is part of the contract (golden corpus):
 *  `ir_version, host, modules[, resources]`, host keys
 *  `os, arch, tags[, libc]` — hostname never crosses into the IR. */
export function emitIr(env: Env, facts: HostFacts, tags: string[]): string {
  const resources = declaredResources();
  const modules: Record<string, IrModule> = {};
  for (const m of env.modules ?? []) {
    if (!m) continue;
    if (typeof m !== "object" || m.__gripsack !== "module") {
      throw new Error(
        `env.modules entries must be module() values — got ${JSON.stringify(
          Array.isArray(m) ? "an array" : typeof m,
        )}`,
      );
    }
    const prev = modules[m.name];
    if (prev) {
      const where = prev.span ? ` (first declared at ${prev.span.file}:${prev.span.line})` : "";
      throw new Error(`duplicate module '${m.name}'${where}`);
    }
    modules[m.name] = m.ir;
  }
  const ir: Record<string, unknown> = {
    ir_version: IR_VERSION,
    host: {
      os: facts.os,
      arch: facts.arch,
      tags,
      ...(facts.libc !== null ? { libc: facts.libc } : {}),
    },
    modules,
  };
  if (resources.length > 0) ir.resources = resources;
  return JSON.stringify(ir, null, 2);
}
