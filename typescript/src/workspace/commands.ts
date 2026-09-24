/** v5 immutable exec/runBash commands, typed arguments and dedent maps. */

import { rejectUnknownFields } from "../fields.ts";
import type { Span } from "../module.ts";
import type {
  ExecSpec,
  RunBashSpec,
  WorkspaceArg,
  WorkspaceArtifactRef,
  WorkspaceCommand,
  WorkspaceExecCommand,
  WorkspaceHostPath,
  WorkspaceLiteral,
  WorkspacePackageCommand,
  WorkspaceRunBashCommand,
} from "./ir.ts";
import {
  asArg,
  asEnv,
  asName,
  asPath,
  asRecord,
  asSelector,
  freezeDeep,
  nodeSpan,
} from "./validate.ts";

/** A literal string — valid as an argv/env argument or a cwd path. */
export function lit(value: string): WorkspaceLiteral {
  if (typeof value !== "string") throw new Error("lit(value) must be a string");
  return freezeDeep({ kind: "literal", value });
}

/** A reference to `<selector>` inside the artifact of output
 *  `<output>` — valid as an argv/env argument or a cwd path. The
 *  selector is `.` (the whole artifact) or a normalized relative
 *  POSIX path; escapes are rejected at declaration. */
export function artifact(output: string, selector: string): WorkspaceArtifactRef {
  return freezeDeep({
    kind: "artifact",
    output: asName(output, "artifact(output)"),
    selector: asSelector(selector, "artifact(selector)"),
  });
}

/** A host filesystem path (command cwd only — never an argv value). */
export function hostPath(path: string): WorkspaceHostPath {
  return freezeDeep({ kind: "host", path: asName(path, "hostPath(path)") });
}

/** A command provided by a declared package: `packageCommand("bash",
 *  "bash")` names the `bash` command of the `bash` package output. */
export function packageCommand(pkg: string, command: string): WorkspacePackageCommand {
  return freezeDeep({
    kind: "package_command",
    package: asName(pkg, "packageCommand(package)"),
    command: asName(command, "packageCommand(command)"),
  });
}

/** Direct argv execution with typed argument references. */
export function exec(spec: ExecSpec): WorkspaceCommand {
  const what = "exec(...)";
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["argv", "env", "cwd", "span"]);
  if (!Array.isArray(spec.argv)) throw new Error(`${what}: argv must be an array`);
  const span = nodeSpan(spec.span, what);
  const argv = spec.argv.map((a, i) => asArg(a, `${what}: argv[${i}]`));
  const env = asEnv(spec.env, `${what}: env`);
  const node: WorkspaceExecCommand = {
    kind: "exec",
    span,
    argv,
    ...(env ? { env } : {}),
    ...(spec.cwd !== undefined ? { cwd: asPath(spec.cwd, `${what}: cwd`) } : {}),
  };
  return freezeDeep(node);
}

function commonPrefix(a: string, b: string): string {
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  return a.slice(0, i);
}

/** Dedent a leading-newline body and map every generated line back to
 *  its source line (A1-06): the core's diagnostics point at the
 *  user's file, not at the dedented script. */
function dedent(body: string, span: Span): { text: string; lineMap?: number[] } {
  if (!body.startsWith("\n")) return { text: body };
  const lines = body.split("\n");
  const start = 1; // the leading newline itself
  let end = lines.length;
  if (end > start && lines[end - 1]!.trim() === "") end -= 1;
  const kept = lines.slice(start, end);
  let prefix: string | undefined;
  for (const l of kept) {
    if (l.trim() === "") continue;
    const ws = l.match(/^[ \t]*/)![0];
    prefix = prefix === undefined ? ws : commonPrefix(prefix, ws);
  }
  const cut = prefix?.length ?? 0;
  return {
    text: kept.map((l) => l.slice(cut)).join("\n"),
    lineMap: kept.map((_, i) => span.line + start + i),
  };
}

/** A literal Bash body run under a pinned package interpreter. The
 *  command is a DESCRIPTION; the enclosing recipe/task/check/hook
 *  admits the execution context. */
export function runBash(spec: RunBashSpec): WorkspaceCommand {
  const what = "runBash(...)";
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["body", "interpreter", "env", "cwd", "span"]);
  if (typeof spec.body !== "string") throw new Error(`${what}: body must be a string`);
  const span = nodeSpan(spec.span, what);
  if (spec.body.includes("${")) {
    throw new Error(
      `${what}: body is literal text — "\${" interpolation is rejected ` +
        `(declared at ${span.file}:${span.line}); pass dynamic values through typed env/argv bindings`,
    );
  }
  const interpreter = asArg(spec.interpreter, `${what}: interpreter`);
  if (interpreter.kind !== "package_command") {
    throw new Error(
      `${what}: interpreter must be a pinned packageCommand("<package>", "<command>") — ` +
        `ambient host shells are never discovered (A1-03)`,
    );
  }
  const { text, lineMap } = dedent(spec.body, span);
  const env = asEnv(spec.env, `${what}: env`);
  const node: WorkspaceRunBashCommand = {
    kind: "run_bash",
    span,
    interpreter,
    body: text,
    ...(env ? { env } : {}),
    ...(spec.cwd !== undefined ? { cwd: asPath(spec.cwd, `${what}: cwd`) } : {}),
    ...(lineMap ? { line_map: lineMap } : {}),
  };
  return freezeDeep(node);
}
