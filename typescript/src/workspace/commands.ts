/** v5 immutable exec/runBash commands, typed arguments and dedent maps. */

import { rejectUnknownFields } from "../fields.ts";
import { DiagnosticError, diagnosticCodes, errorAt } from "../diagnostic.ts";
import type { Span } from "../module.ts";
import type {
  BashBody,
  ExecSpec,
  RunBashSpec,
  WorkspaceArg,
  WorkspaceArtifactRef,
  WorkspaceExecCommand,
  WorkspaceHostPath,
  WorkspaceLiteral,
  WorkspacePackageCommand,
  WorkspacePath,
  WorkspaceRunBashCommand,
} from "./ir.ts";
import {
  asArg,
  asEnv,
  asName,
  asPath,
  asRecord,
  asSelector,
  asSpan,
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

/** Fluent exec authoring; each method returns a new immutable branch.
 *  `build()` uses the same normalization as object-form `exec({ argv })`. */
export interface ExecBuilder {
  arg(value: WorkspaceArg): ExecBuilder;
  env(name: string, value: WorkspaceArg): ExecBuilder;
  cwd(path: WorkspacePath): ExecBuilder;
  build(): WorkspaceExecCommand;
}

function execBuilder(spec: ExecSpec): ExecBuilder {
  return Object.freeze({
    arg(value: WorkspaceArg): ExecBuilder {
      const arg = freezeDeep(asArg(value, "exec(...).arg"));
      return execBuilder({ ...spec, argv: [...spec.argv, arg] });
    },
    env(name: string, value: WorkspaceArg): ExecBuilder {
      const key = asName(name, "exec(...).env(name)");
      const entry = asEnv({ [key]: value }, "exec(...).env")!;
      return execBuilder({ ...spec, env: { ...spec.env, ...freezeDeep(entry) } });
    },
    cwd(path: WorkspacePath): ExecBuilder {
      return execBuilder({ ...spec, cwd: freezeDeep(asPath(path, "exec(...).cwd")) });
    },
    build(): WorkspaceExecCommand {
      return exec(spec);
    },
  });
}

/** Direct argv execution: object form or an immutable fluent builder.
 *  Only typed arguments cross the argv/env boundary, never shell text. */
export function exec(spec: ExecSpec): WorkspaceExecCommand;
export function exec(program: WorkspaceArg): ExecBuilder;
export function exec(spec: ExecSpec | WorkspaceArg): WorkspaceExecCommand | ExecBuilder {
  const what = "exec(...)";
  const value = asRecord(spec, what);
  if (value.kind !== undefined) {
    const program = freezeDeep(asArg(spec, `${what}: program`));
    return execBuilder({ argv: [program], span: nodeSpan(undefined, what) });
  }
  rejectUnknownFields(what, value, ["argv", "env", "cwd", "span"]);
  const fields = spec as ExecSpec;
  if (!Array.isArray(fields.argv)) throw new Error(`${what}: argv must be an array`);
  const span = nodeSpan(fields.span, what);
  const argv = fields.argv.map((a, i) => asArg(a, `${what}: argv[${i}]`));
  const env = asEnv(fields.env, `${what}: env`);
  const node: WorkspaceExecCommand = {
    kind: "exec",
    span,
    argv,
    ...(env ? { env } : {}),
    ...(fields.cwd !== undefined ? { cwd: asPath(fields.cwd, `${what}: cwd`) } : {}),
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
  if (!body.startsWith("\n")) {
    if (!body.includes("\n")) return { text: body };
    return { text: body, lineMap: body.split("\n").map((_, i) => span.line + i) };
  }
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

/** Capture the opening source line of a literal Bash template.
 *  Values in a JS interpolation are evaluated before this tag is
 *  called, so only literal text belongs here; static authoring checks
 *  must still guard potentially effectful expressions. */
export function bashBody(parts: TemplateStringsArray, ...values: unknown[]): BashBody {
  const span = nodeSpan(undefined, "bashBody`...`");
  if (values.length !== 0 || parts.raw.length !== 1) {
    const before = parts.raw[0] ?? "";
    const line = span.line + before.split("\n").length - 1;
    throw new DiagnosticError({
      code: diagnosticCodes.invalidWorkspaceValue,
      severity: "error",
      message: "bashBody: interpolation is not a literal Bash body",
      labels: [{ span: { file: span.file, line }, note: "interpolation evaluated here" }],
      help: "pass dynamic values through typed env/argv bindings",
    });
  }
  const text = parts.raw[0]!;
  rejectBashInterpolation(text, span, "bashBody`...`");
  return freezeDeep({ text, span });
}

function rejectBashInterpolation(body: string, span: Span, where: string): void {
  const site = body.indexOf("${");
  if (site === -1) return;
  const line = span.line + body.slice(0, site).split("\n").length - 1;
  throw new DiagnosticError({
    code: diagnosticCodes.invalidWorkspaceValue,
    severity: "error",
    message: `${where}: body is literal text — "\${" interpolation is rejected`,
    labels: [{ span: { file: span.file, line }, note: "interpolation rejected here" }],
    help: "pass dynamic values through typed env/argv bindings",
  });
}

/** A literal Bash body run under a pinned package interpreter. The
 *  command is a DESCRIPTION; the enclosing recipe/task/check/hook
 *  admits the execution context. */
export function runBash(spec: RunBashSpec): WorkspaceRunBashCommand {
  const what = "runBash(...)";
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["body", "interpreter", "env", "cwd", "span"]);
  const span = nodeSpan(spec.span, what);
  let body: string;
  let bodySpan: Span;
  if (typeof spec.body === "string") {
    body = spec.body;
    bodySpan = span;
    if (body.includes("\n")) {
      throw errorAt(
        diagnosticCodes.invalidWorkspaceValue,
        `${what}: multiline body needs bashBody\`...\` to preserve original source lines`,
        span,
        "body declared here",
      );
    }
  } else {
    const source = asRecord(spec.body, `${what}: body`);
    rejectUnknownFields(`${what}: body`, source, ["text", "span"]);
    if (typeof source.text !== "string") throw new Error(`${what}: body.text must be a string`);
    body = source.text;
    bodySpan = asSpan(source.span, `${what}: body`);
  }
  rejectBashInterpolation(body, bodySpan, what);
  const interpreter = asArg(spec.interpreter, `${what}: interpreter`);
  if (interpreter.kind !== "package_command") {
    throw errorAt(
      diagnosticCodes.badWorkspaceContext,
      `${what}: interpreter must be a pinned packageCommand("<package>", "<command>") — ` +
        `ambient host shells are never discovered (A1-03)`,
      span,
      "interpreter declared here",
    );
  }
  const { text, lineMap } = dedent(body, bodySpan);
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

/** A body is required before a Bash command can be constructed. */
export interface BashBuilder {
  body(body: RunBashSpec["body"]): BashCommandBuilder;
}

/** Immutable Bash command authoring after a literal body is supplied. */
export interface BashCommandBuilder {
  env(name: string, value: WorkspaceArg): BashCommandBuilder;
  cwd(path: WorkspacePath): BashCommandBuilder;
  build(): WorkspaceRunBashCommand;
}

function bashCommandBuilder(spec: RunBashSpec): BashCommandBuilder {
  return Object.freeze({
    env(name: string, value: WorkspaceArg): BashCommandBuilder {
      const key = asName(name, "bash(...).env(name)");
      const entry = asEnv({ [key]: value }, "bash(...).env")!;
      return bashCommandBuilder({ ...spec, env: { ...spec.env, ...freezeDeep(entry) } });
    },
    cwd(path: WorkspacePath): BashCommandBuilder {
      return bashCommandBuilder({ ...spec, cwd: freezeDeep(asPath(path, "bash(...).cwd")) });
    },
    build(): WorkspaceRunBashCommand {
      return runBash(spec);
    },
  });
}
/** Start a Bash command with a declared package interpreter, never a
 *  host-shell name. `body(...).build()` lowers through `runBash({…})`. */
export function bash(interpreter: WorkspacePackageCommand): BashBuilder {
  const span = nodeSpan(undefined, "bash(...)");
  const pin = asArg(interpreter, "bash(interpreter)");
  if (pin.kind !== "package_command") {
    throw errorAt(
      diagnosticCodes.badWorkspaceContext,
      "bash(interpreter) requires a declared packageCommand — ambient host shells are never discovered (A1-03)",
      span,
      "interpreter declared here",
    );
  }
  const stablePin = freezeDeep(pin);
  return Object.freeze({
    body(body: RunBashSpec["body"]): BashCommandBuilder {
      return bashCommandBuilder({ interpreter: stablePin, body: freezeDeep(body), span });
    },
  });
}
