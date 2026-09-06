/** Dependency purposes (0039): the DSL and IR v2 use the same `for` field. */
import { callerSpan, type Span } from "./module.ts";

export type Edge = "runtime" | "build";

export interface DepOptions {
  for?: Edge;
}

export interface Dependency {
  module: string;
  for: Edge;
  span?: Span;
}

/** Build-only dependencies supply PATH and GRIP_DEP_* to build steps, not HOME. */
export function dep(module: string, opts: DepOptions = {}): Dependency {
  if (typeof opts !== "object" || opts === null || Array.isArray(opts)) {
    throw new Error(`dep(${JSON.stringify(module)}): expected an options object { for: "build" | "runtime" }`);
  }
  for (const key of Object.keys(opts)) {
    if (key !== "for") {
      throw new Error(`dep(${JSON.stringify(module)}): unknown option ${JSON.stringify(key)} (known: for)`);
    }
  }
  // The typed core validates values, including JS/cast inputs, with E122 and
  // this source span. Do not turn an invalid purpose into a runtime edge.
  const edge = opts.for === undefined ? "runtime" : opts.for;
  const span = callerSpan();
  return { module, for: edge, ...(span ? { span } : {}) };
}
