/** The module graph: registry and IR emission (0001 §3.2). */

import { currentFacts } from "./facts.js";
import { collectPipeline, Module } from "./module.js";
import type { IrModule, Span } from "./module.js";
import { declaredResources } from "./resources.js";

export const IR_VERSION = 1;

const GRAPH = new Map<string, IrModule>();
const CLASSES = new Map<string, { ctor: new () => Module; span?: Span }>();

/** Register a module. Duplicate names throw with both declaration
 *  sites (the IR map can only ever hold one). */
export function register(name: string, ir: IrModule): void {
  const prevSpan = GRAPH.get(name)?.span ?? CLASSES.get(name)?.span;
  if (GRAPH.has(name) || CLASSES.has(name)) {
    const where = prevSpan ? ` (first declared at ${prevSpan.file}:${prevSpan.line})` : "";
    throw new Error(`duplicate module '${name}'${where}`);
  }
  GRAPH.set(name, ir);
}

/** Register a class-style module; instantiated lazily at emit time. */
export function registerClass(name: string, ctor: new () => Module, span?: Span): void {
  if (GRAPH.has(name) || CLASSES.has(name)) {
    throw new Error(`duplicate module '${name}'`);
  }
  const entry: { ctor: new () => Module; span?: Span } = { ctor };
  if (span) entry.span = span;
  CLASSES.set(name, entry);
}

/** Drop all registered modules (test isolation). */
export function clearGraph(): void {
  GRAPH.clear();
  CLASSES.clear();
}

/** Serialize the registered graph as IR JSON. */
export function emitIr(tags: string[] = []): string {
  const resources = declaredResources();
  const modules: Record<string, IrModule> = Object.fromEntries(GRAPH);
  for (const [name, { ctor, span }] of CLASSES) {
    const instance = new ctor();
    const { steps, verify } = collectPipeline(instance);
    const ir: IrModule = { steps };
    if (verify) ir.verify = verify;
    if (span) ir.span = span;
    modules[name] = ir;
  }
  const ir: Record<string, unknown> = {
    ir_version: IR_VERSION,
    host: currentFacts(tags),
    modules,
  };
  if (resources.length > 0) ir.resources = resources;
  return JSON.stringify(ir, null, 2);
}
