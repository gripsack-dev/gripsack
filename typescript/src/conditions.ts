/** Conditional logic over core-injected facts (0001 §5, 0013 D4).
 *  Pure functions: they take the fact view as an argument instead of
 *  reading a global — `when(cond, ctx)` inside a `defineEnv`
 *  function, where ctx is handed in. */

import type { HostFacts } from "./facts.ts";

/** What conditionals evaluate against. The eval context satisfies
 *  this structurally, so `when(cond, ctx)` and `hasTag(t, ctx)` work
 *  on the context directly. */
export interface FactView {
  facts: HostFacts;
  tags: string[];
}

export interface Condition {
  os?: string;
  arch?: string;
  /** `null` matches "undetectable". */
  libc?: string | null;
  tags?: string[];
  notTags?: string[];
}

/** Build a condition over host facts — evaluate directly:
 *  `when({ os: "linux" }, ctx) && module(...)`, or gate a whole entry
 *  on it. Falsy-drop in `defineEnv`'s modules array does the rest. */
export function when(cond: Condition, view: FactView): boolean {
  const f = view.facts;
  if (cond.os !== undefined && f.os !== cond.os) return false;
  if (cond.arch !== undefined && f.arch !== cond.arch) return false;
  if (cond.libc !== undefined && (f.libc ?? null) !== cond.libc) return false;
  if (cond.tags?.some((t) => !view.tags.includes(t))) return false;
  if (cond.notTags?.some((t) => view.tags.includes(t))) return false;
  return true;
}

/** True when the merged tag set contains `tag`. */
export function hasTag(tag: string, view: FactView): boolean {
  return view.tags.includes(tag);
}
