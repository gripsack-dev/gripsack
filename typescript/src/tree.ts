/** Tree entries — directory-shaped config deploys via eval-time
 *  expansion (the IR stays per-file). Adding or removing files in the
 *  directory is picked up at the next eval; files dropped from the
 *  tree are pruned at apply.
 *
 *  ```ts
 *  module("zed", { config: { ...tree("configs/zed", "~/.config/zed") } });
 *  ```
 */

import { readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import type { Dest, Ownership } from "./entries.ts";

export function tree(
  src: string,
  to: string,
  mode: Ownership = "tracked_copy",
): Record<string, Dest> {
  const entries: Record<string, Dest> = {};
  const walk = (dir: string): void => {
    for (const name of readdirSync(dir).sort()) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        walk(path);
      } else if (statSync(path).isFile()) {
        const rel = relative(src, path).split("\\").join("/");
        entries[`${src}/${rel}`] = { to: `${to}/${rel}`, mode };
      }
    }
  };
  walk(src);
  return entries;
}
