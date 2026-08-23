/** Conditional modules: `when` predicates over host facts (0001 §5).
 *  The `facts` object is populated by the eval runner after the host
 *  entrypoint; modules read it freely. */

export interface Facts {
  os: string;
  arch: string;
  libc?: string | undefined;
  tags: string[];
}

import { currentFacts } from "./facts.js";

const auto = currentFacts();
let live: Facts = { os: auto.os, arch: auto.arch, libc: auto.libc, tags: auto.tags };

/** The shared host facts — set by the runner, read by modules. */
export const facts: Facts = {
  get os() {
    return live.os;
  },
  get arch() {
    return live.arch;
  },
  get libc() {
    return live.libc;
  },
  get tags() {
    return live.tags;
  },
};

/** True when the host declared `tag`. */
export function hasTag(tag: string): boolean {
  return live.tags.includes(tag);
}

/** @internal Called by the eval runner once tags are known. */
export function setTags(tags: string[]): void {
  live = { ...live, tags };
}

export interface Condition {
  os?: string;
  arch?: string;
  libc?: string | undefined;
  tags?: string[];
  notTags?: string[];
}

/** Build a condition over host facts — data style via `moduleIf` /
 *  `defineIf`, or evaluate directly: `when({ os: "linux" })`. */
export function when(cond: Condition): boolean {
  if (cond.os !== undefined && live.os !== cond.os) return false;
  if (cond.arch !== undefined && live.arch !== cond.arch) return false;
  if (cond.libc !== undefined && live.libc !== cond.libc) return false;
  if (cond.tags?.some((t) => !live.tags.includes(t))) return false;
  if (cond.notTags?.some((t) => live.tags.includes(t))) return false;
  return true;
}
