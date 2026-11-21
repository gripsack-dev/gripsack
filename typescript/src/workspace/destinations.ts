import { DiagnosticError, diagnosticCodes } from "../diagnostic.ts";
import type { WorkspaceFile, WorkspaceOutputNode } from "./ir.ts";

/** IR10 compares literal declarations, not physical paths on the evaluating host.
 * Tree leaves, home expansion and symlink aliases are resolved by the core. */
export function checkLiteralDestinations(outputs: Iterable<WorkspaceOutputNode>): void {
  const paths = new Map<string, WorkspaceFile[]>();
  for (const node of outputs) {
    if (node.kind !== "profile") continue;
    for (const file of node.files ?? []) {
      if (file.source?.kind === "tree" || file.source?.kind === "repo_tree") continue;
      const key = file.destination.path.toLowerCase();
      const owners = paths.get(key);
      if (owners) owners.push(file);
      else paths.set(key, [file]);
    }
  }
  for (const [path, files] of paths) {
    if (files.length < 2) continue;
    const markers = new Map<string, WorkspaceFile[]>();
    for (const file of files) {
      if (file.destination.kind !== "managed_block") conflict(path, files);
      const key = file.destination.marker.toLowerCase();
      const owners = markers.get(key);
      if (owners) owners.push(file);
      else markers.set(key, [file]);
    }
    for (const [marker, owners] of markers) {
      if (owners.length > 1) conflict(path, owners, marker);
    }
  }
}

function conflict(path: string, owners: WorkspaceFile[], marker?: string): never {
  const what = marker === undefined ? `destination '${path}'` : `managed block '${marker}' at destination '${path}'`;
  const spans = owners.map((file) => file.span).sort((a, b) =>
    a.file < b.file ? -1 : a.file > b.file ? 1 : a.line - b.line || (a.col ?? 0) - (b.col ?? 0)
  );
  throw new DiagnosticError({
    code: diagnosticCodes.duplicateDestination,
    severity: "error",
    message: `workspace profile files declare ${what} ${owners.length} times (case-insensitive destination comparison)`,
    labels: spans.map((span, index) => ({ span, note: index === 0 ? "first declared here" : "also declared here" })),
    help: "split the destination, or drop one declaration",
  });
}
