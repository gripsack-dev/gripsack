import { diagnosticCodes, errorAt } from "../diagnostic.ts";
import type { WorkspaceOutputNode } from "./ir.ts";

/** A file subject exists only at an explicit attachment to its owning profile. */
export function fileCheckAttachments(catalog: ReadonlyMap<string, WorkspaceOutputNode>): ReadonlySet<string> {
  const attached = new Set<string>();
  for (const profile of catalog.values()) {
    if (profile.kind !== "profile") continue;
    for (const file of profile.files ?? []) {
      for (const binding of file.checks ?? []) {
        const check = catalog.get(binding.check);
        if (check?.kind !== "check") continue; // Shared reference admission diagnoses this.
        if (check.subject !== profile.name) {
          throw errorAt(diagnosticCodes.badWorkspaceContext,
            "a profile file check must name its owning profile as the check subject",
            binding.span, "file check attached here");
        }
        attached.add(check.name);
        if (binding.subject === "source" && file.source === undefined) {
          throw errorAt(diagnosticCodes.badWorkspaceContext,
            "a source check requires a captured file source", binding.span, "file check attached here");
        }
        if (binding.subject === "rendered" && binding.stage !== "pre_flip") {
          throw errorAt(diagnosticCodes.badWorkspaceContext,
            "rendered file validation must run before deployment at pre_flip",
            binding.span, "file check attached here");
        }
      }
    }
  }
  for (const output of catalog.values()) {
    if (output.kind === "task") {
      for (const name of output.checks ?? []) {
        if (attached.has(name)) {
          throw errorAt(diagnosticCodes.badWorkspaceContext,
            "a profile-bound file check cannot serve as a task postcondition",
            output.span, "check consumed in a different lifecycle here");
        }
      }
    }
  }
  return attached;
}
