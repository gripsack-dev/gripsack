/** Workspace declarations (0052 A1) — split into cohesive modules
 *  (plan/0052 §3 ~400-line review): ir.ts (wire types), validate.ts
 *  (shared runtime guards), commands.ts (exec/runBash + dedent),
 *  files.ts (origin/content/destination axes), outputs.ts (the nine
 *  output constructors + workspace entrypoint), emit.ts (reference/
 *  cycle admission + the v4 envelope). ../workspace.ts is the
 *  supported re-export surface. */

import { rejectUnknownFields } from "../fields.ts";
import type {
  WorkspaceContent,
  WorkspaceDestination,
  WorkspaceFile,
  WorkspaceFileSpec,
  WorkspaceSource,
} from "./ir.ts";
import {
  asContent,
  asDestination,
  asName,
  asRecord,
  asSelector,
  asSource,
  freezeDeep,
  nodeSpan,
} from "./validate.ts";


// file-declaration axes (origin / content / destination — orthogonal)

/** A typed repository file origin. */
export function repoFile(path: string): WorkspaceSource {
  return freezeDeep({ kind: "repo_file", path: asName(path, "repoFile(path)") });
}

/** A file inside another output's artifact. */
export function artifactFile(output: string, selector: string): WorkspaceSource {
  return freezeDeep({
    kind: "artifact_file",
    output: asName(output, "artifactFile(output)"),
    selector: asSelector(selector, "artifactFile(selector)"),
  });
}

/** Content is exactly the origin's bytes. */
export function identity(): WorkspaceContent {
  return freezeDeep({ kind: "identity" });
}

/** Inline literal content — the only content that needs no origin. */
export function literalText(text: string): WorkspaceContent {
  if (typeof text !== "string") throw new Error("literalText(text) must be a string");
  return freezeDeep({ kind: "literal", text });
}

/** Content rendered from a template with bound variables. */
export function templateText(
  template: string,
  variables: Record<string, string>,
): WorkspaceContent {
  if (typeof template !== "string") throw new Error("templateText(template) must be a string");
  const vars = asRecord(variables, "templateText(...): variables");
  for (const [k, v] of Object.entries(vars)) {
    if (typeof v !== "string") {
      throw new Error(`templateText(...): variables["${k}"] must be a string`);
    }
  }
  return freezeDeep({ kind: "template", template, variables });
}

/** Store-owned, read-only destination; edits go through the declaration. */
export function symlinkTo(path: string): WorkspaceDestination {
  return freezeDeep({ kind: "symlink", path: asName(path, "symlinkTo(path)") });
}

/** Copied destination; drift detected on next apply. */
export function trackedCopyTo(path: string): WorkspaceDestination {
  return freezeDeep({ kind: "tracked_copy", path: asName(path, "trackedCopyTo(path)") });
}

/** Managed block merged into a file other tools also write. */
export function managedBlock(path: string, marker: string): WorkspaceDestination {
  return freezeDeep({
    kind: "managed_block",
    path: asName(path, "managedBlock(path)"),
    marker: asName(marker, "managedBlock(marker)"),
  });
}

/** One profile file: origin, content and destination policy compose
 *  independently. */
export function file(spec: WorkspaceFileSpec): WorkspaceFile {
  const what = "file(...)";
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["source", "content", "destination", "span"]);
  const span = nodeSpan(spec.span, what);
  const content = asContent(spec.content, `${what}: content`);
  if (spec.source === undefined && content.kind !== "literal") {
    throw new Error(
      `${what}: source is required unless content is literalText(...) — ` +
        `a ${content.kind} content has no bytes without an origin`,
    );
  }
  const node: WorkspaceFile = {
    span,
    ...(spec.source !== undefined ? { source: asSource(spec.source, `${what}: source`) } : {}),
    content,
    destination: asDestination(spec.destination, `${what}: destination`),
  };
  return freezeDeep(node);
}
