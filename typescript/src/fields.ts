/** Runtime unknown-field rejection (0035 F3), shared by `module()`
 *  and the workspace constructors: the CLI evaluates authoring code
 *  without a type-checker, so a typo'd field must not silently lower
 *  to an empty desired state (a `confg:` became a prune). Runtime
 *  rejection is the backstop for JS callers, casts, and generated
 *  objects. @internal */

function editDistance(a: string, b: string): number {
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const cur: number[] = [i];
    const prevRow: number[] = prev;
    for (let j = 1; j <= b.length; j++) {
      cur[j] = Math.min(
        prevRow[j]! + 1,
        cur[j - 1]! + 1,
        prevRow[j - 1]! + (a[i - 1] === b[j - 1] ? 0 : 1),
      );
    }
    prev = cur;
  }
  return prev[b.length]!;
}

/** Throw when `spec` carries a field outside `known`, with a
 *  did-you-mean when a known field is within edit distance 2.
 *  `what` names the constructor call, e.g. `module("demo")`. */
export function rejectUnknownFields(
  what: string,
  spec: object,
  known: readonly string[],
): void {
  for (const key of Object.keys(spec)) {
    if (!known.includes(key)) {
      const closest = known
        .map((k) => [k, editDistance(key, k)] as const)
        .filter(([, d]) => d <= 2)
        .sort((a, b) => a[1] - b[1])[0];
      throw new Error(
        `${what}: unknown field "${key}"` +
          (closest ? ` — did you mean "${closest[0]}"?` : "") +
          ` (known: ${known.join(", ")})`,
      );
    }
  }
}
