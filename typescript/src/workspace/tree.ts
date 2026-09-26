/** Bounded eval-time tree expansion for workspace profile files
 * (0052 §2.2 FileDecl tree origin; Epic A §3.3): walk a captured
 * repository directory and return explicit per-file declarations.
 *
 * The IR stays per-file — no directory-shaped ownership is invented,
 * and the tree never claims unrelated children: adding or removing a
 * file changes exactly that file's declaration at the next eval.
 * Enumeration is stable (sorted at each depth), every entry carries the
 * caller's declaration span, and symlinks/special files are rejected
 * rather than followed so a repo link can never pull outside content
 * into the captured inventory. Artifact-side trees (an output not yet
 * realized) cannot be enumerated at eval time and remain A2-owned. */

import { lstatSync, readdirSync } from "node:fs";
import { file, identity, repoFile, symlinkTo, trackedCopyTo } from "./files.ts";
import type { WorkspaceFile } from "./ir.ts";
import { asDestinationPath } from "./validate.ts";

export interface TreeFilesOptions {
  /** Restrict expansion to these subtrees/paths (relative to `src`,
   *  normalized). Omit to admit the whole directory. */
  include?: string[];
  /** Skip these subtrees/paths; wins over `include`. */
  exclude?: string[];
  /** Destination policy applied to every entry (default tracked_copy). */
  mode?: "symlink" | "tracked_copy";
  /** Admission cap on expanded entries (default and maximum 10 000). */
  maxEntries?: number;
}

const MAX_TREE_ENTRIES = 10_000;

/** A repo-relative directory: `"."` (the repo root) or normalized
 *  relative POSIX segments. */
function asRepoDirPath(path: string, where: string): string {
  if (
    path === "." ||
    !(path.startsWith("/") || path.includes("\\") || path.includes("\0") ||
      path.split("/").some((part) => part === "" || part === "." || part === ".."))
  ) {
    return path;
  }
  throw new Error(
    `${where}: repository directory must be "." or a normalized relative POSIX path ` +
      `(no leading "/", backslash, NUL, empty, "." or ".." segments) — got ${JSON.stringify(path)}`,
  );
}

/** Segment-prefix match: "a/b" covers "a/b" and "a/b/c", never "a/bc". */
function covered(segments: readonly string[], pattern: readonly string[]): boolean {
  return pattern.length <= segments.length &&
    pattern.every((part, index) => part === segments[index]);
}

export function treeFiles(
  src: string,
  to: string,
  options: TreeFilesOptions = {},
): WorkspaceFile[] {
  const what = "treeFiles(src, to)";
  const root = asRepoDirPath(src, `${what}: src`);
  const mode = options.mode ?? "tracked_copy";
  const maxEntries = Math.min(options.maxEntries ?? MAX_TREE_ENTRIES, MAX_TREE_ENTRIES);
  const patterns = (list: string[] | undefined, label: string): string[][] =>
    (list ?? []).map((pattern) => {
      if (pattern === ".") {
        throw new Error(`${what}: ${label} pattern "." is redundant — omit it to admit the whole tree`);
      }
      return asRepoDirPath(pattern, `${what}: ${label}`).split("/");
    });
  const include = patterns(options.include, "include");
  const exclude = patterns(options.exclude, "exclude");
  // Validate the destination shape even for an empty tree.
  asDestinationPath(to, `${what}: to`);
  const policy = mode === "symlink" ? symlinkTo : trackedCopyTo;
  const rootKind = lstatSync(root, { throwIfNoEntry: false });
  if (!rootKind || !rootKind.isDirectory()) {
    throw new Error(
      `${what}: src ${JSON.stringify(src)} must be an existing directory inside the evaluated repository`,
    );
  }

  const atRoot = root === ".";
  const files: WorkspaceFile[] = [];
  const walk = (relative: readonly string[]): void => {
    const directory = atRoot
      ? (relative.length === 0 ? "." : relative.join("/"))
      : [root, ...relative].join("/");
    for (const name of readdirSync(directory).sort()) {
      const entryPath = `${directory}/${name}`;
      const kind = lstatSync(entryPath);
      if (kind.isDirectory()) {
        walk([...relative, name]);
        continue;
      }
      if (!kind.isFile()) {
        throw new Error(
          `${what}: ${JSON.stringify(entryPath)} is not a regular file or directory ` +
            `(symlinks and special entries are never followed into a captured tree)`,
        );
      }
      const segments = [...relative, name];
      if (exclude.some((pattern) => covered(segments, pattern))) continue;
      if (include.length > 0 && !include.some((pattern) => covered(segments, pattern))) {
        continue;
      }
      if (files.length === maxEntries) {
        throw new Error(
          `${what}: expanding ${JSON.stringify(src)} exceeded the ${maxEntries} entry cap ` +
            `(pass a smaller tree or narrow include/exclude)`,
        );
      }
      const relativePath = segments.join("/");
      files.push(
        file({
          source: repoFile(atRoot ? relativePath : `${root}/${relativePath}`),
          content: identity(),
          destination: policy(`${to}/${relativePath}`),
        }),
      );
    }
  };
  walk([]);
  return files;
}
