import { emitWorkspaceIr } from "../src/advanced.ts";
import { check, exec, file, identity, lit, outputPath, profile, repoFile,
  sourcePath, task, trackedCopyTo, workspace } from "../src/workspace.ts";
import type { WorkspaceArg, WorkspaceFileCheck, WorkspaceOutput } from "../src/workspace.ts";
import { thrownDiagnostic } from "./diagnostic.ts";

const facts = { os: "linux", arch: "x86_64", libc: null, hostname: "fixture" } as const;
function declarations(argument: WorkspaceArg = sourcePath("."),
  stage: WorkspaceFileCheck["stage"] = "pre_flip", attached = true): WorkspaceOutput[] {
  return [
    check("syntax", { subject: "personal", run: exec({ argv: [lit("/bin/true"), argument] }) }),
    profile("personal", { files: [file({ source: repoFile("config"), content: identity(),
      destination: trackedCopyTo("~/.config/tool"),
      checks: attached ? [{ check: "syntax", subject: "rendered", stage,
        span: { file: "files.ts", line: 4 } }] : [],
    })] }),
  ];
}
function emit(outputs: WorkspaceOutput[]): string {
  return emitWorkspaceIr(workspace({ outputs }), facts);
}

Deno.test("file subjects are scoped to the attached leaf, not adjacent or writable paths", () => {
  for (const argument of [sourcePath("sibling"), outputPath(".")]) {
    thrownDiagnostic(() => emit(declarations(argument)), "E128");
  }
  thrownDiagnostic(() => emit(declarations(sourcePath("."), "pre_flip", false)), "E128");
  thrownDiagnostic(() => emit(declarations(sourcePath("."), "post_activate")), "E128");
});

Deno.test("profile-file validation does not become an invocation postcondition", () => {
  const outputs = declarations();
  outputs.push(task("misuse", { steps: [exec({ argv: [lit("/bin/true")] })], checks: ["syntax"] }));
  thrownDiagnostic(() => emit(outputs), "E128");
});
