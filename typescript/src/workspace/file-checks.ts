import { diagnosticCodes, errorAt } from "../diagnostic.ts";
import type { CheckNode, WorkspaceArg, WorkspaceOutputNode } from "./ir.ts";

/** A file subject exists only at an explicit attachment to its owning profile. */
export function checkFileBindings(catalog: ReadonlyMap<string, WorkspaceOutputNode>): void {
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
    if (output.kind === "recipe" || output.kind === "task") {
      for (const name of output.checks ?? []) {
        if (attached.has(name)) {
          throw errorAt(diagnosticCodes.badWorkspaceContext,
            "a profile-bound file check cannot serve as a recipe validator or task postcondition",
            output.span, "check consumed in a different lifecycle here");
        }
      }
    }
    if (output.kind !== "check" || catalog.get(output.subject)?.kind !== "profile") continue;
    const fileSubject = attached.has(output.name);
    const argument = (value: WorkspaceArg): void => {
      if (value.kind === "output" ||
          (value.kind === "source" && (!fileSubject || value.selector !== "."))) {
        reject(output, "a file check may use sourcePath(\".\") only for its explicitly attached file, never writable output");
      }
    };
    if (output.run.kind === "exec") output.run.argv.forEach(argument);
    else argument(output.run.interpreter);
    Object.values(output.run.env ?? {}).forEach(argument);
    if (output.run.cwd?.kind === "source" || output.run.cwd?.kind === "output") {
      reject(output, "an attached file cannot supply a working directory or writable output");
    }
  }
}

function reject(check: CheckNode, message: string): never {
  throw errorAt(diagnosticCodes.badWorkspaceContext, message, check.run.span, "file subject used here");
}
