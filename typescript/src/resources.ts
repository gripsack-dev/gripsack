/**
 * Declared resources — closing the namespace so typos are errors (0007 §4).
 *
 * Resources are named, host-global mutexes (or pools) that steps acquire
 * before running. The core has built-ins for known contention domains
 * (`network`, `pixi-lock`, `cargo-lock`); anything else must be declared
 * in your env repo.
 *
 * @example
 * ```ts
 * import { resource, step } from "@gripsack/core";
 *
 * const PIXI = resource("pixi.lock");
 * step("sync", { kind: "custom_shell", script: "pixi install" },
 *      { resources: ["pixi.lock"] });
 *
 * // typos throw at eval time, before the core ever sees your IR:
 * step("bad", { kind: "custom_shell", script: "true" },
 *      { resources: ["cargo.lokc"] }); // Error: unknown resource 'cargo.lokc'
 * ```
 */

/** Built-in contention domains the core knows how to serialize or
 *  throttle. Mirrors `KNOWN_RESOURCES` in `crates/gripsack-ir` — keep
 *  the two in sync (IR changes touch all sides). */
export const CORE_RESOURCES: ReadonlySet<string> = new Set([
  "network",
  "pixi-lock",
  "cargo-lock",
]);

/** A declared resource marker. Create with {@link resource}. */
export interface Resource {
  name: string;
}

const REGISTRY = new Map<string, Resource>();

/** Declare a resource and return its marker. */
export function resource(name: string): Resource {
  if (!name) throw new Error("resource name must not be empty");
  const r: Resource = { name };
  REGISTRY.set(name, r);
  return r;
}

/** All resources declared so far in this eval, sorted by name. */
export function declaredResources(): Resource[] {
  return [...REGISTRY.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** Drop all declared resources (test isolation). */
export function clearResources(): void {
  REGISTRY.clear();
}

/** @internal Throw if any ref is neither declared nor built-in. */
export function validateResourceRefs(refs: string[], owner: string): void {
  for (const ref of refs) {
    if (!REGISTRY.has(ref) && !CORE_RESOURCES.has(ref)) {
      const known = [...REGISTRY.keys(), ...CORE_RESOURCES].sort().join(", ");
      throw new Error(
        `unknown resource '${ref}' in ${owner} — declare it first with resource('${ref}'). Known: ${known}`,
      );
    }
  }
}
