/** The module graph: registry and IR emission (0001 §3.2). */

import { currentFacts } from "./facts.js";
import type { IrModule } from "./module.js";
import { declaredResources } from "./resources.js";

export const IR_VERSION = 1;

const GRAPH = new Map<string, IrModule>();

export function register(name: string, ir: IrModule): void {
  GRAPH.set(name, ir);
}

/** Drop all registered modules (test isolation). */
export function clearGraph(): void {
  GRAPH.clear();
}

/** Serialize the registered graph as IR JSON. */
export function emitIr(tags: string[] = []): string {
  const resources = declaredResources();
  const ir: Record<string, unknown> = {
    ir_version: IR_VERSION,
    host: currentFacts(tags),
    modules: Object.fromEntries(GRAPH),
  };
  if (resources.length > 0) ir.resources = resources;
  return JSON.stringify(ir, null, 2);
}
