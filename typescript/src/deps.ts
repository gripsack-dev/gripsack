/** Module dependency edges (0001 §3.1). */

export type Edge = "runtime" | "build";

export interface Dependency {
  module: string;
  edge: Edge;
}

/** `edge: "build"` = ephemeral, build-only. */
export const dep = (module: string, edge: Edge = "runtime"): Dependency => ({
  module,
  edge,
});
