/** Runtime structural guards for v5 workspace authoring values. */

import { rejectUnknownFields } from "../fields.ts";
import { DiagnosticError, diagnosticCodes } from "../diagnostic.ts";
import type { Fetch } from "../fetch.ts";
import { callerSpan } from "../module.ts";
import type { Span } from "../module.ts";
import type {
  WorkspaceArg,
  WorkspaceCalendar,
  WorkspaceCommand,
  WorkspaceContent,
  WorkspaceDestination,
  WorkspaceFile,
  WorkspacePath,
  WorkspaceProducer,
  WorkspaceSource,
} from "./ir.ts";

// ---------------------------------------------------------------------------

export function asRecord(v: unknown, where: string): Record<string, unknown> {
  if (typeof v !== "object" || v === null || Array.isArray(v)) {
    throw new Error(`${where} must be an object`);
  }
  return v as Record<string, unknown>;
}

export function asName(v: unknown, where: string): string {
  if (typeof v !== "string" || v === "") {
    throw new Error(`${where} must be a non-empty string`);
  }
  return v;
}

export function asSpan(v: unknown, where: string): Span {
  const rec = asRecord(v, `${where}: span`);
  rejectUnknownFields(`${where}: span`, rec, ["file", "line", "col"]);
  if (typeof rec.file !== "string" || rec.file === "") {
    throw new Error(`${where}: span.file must be a non-empty string`);
  }
  if (!Number.isInteger(rec.line) || (rec.line as number) < 1) {
    throw new Error(`${where}: span.line must be an integer >= 1`);
  }
  if (
    rec.col !== undefined &&
    (!Number.isInteger(rec.col) || (rec.col as number) < 1)
  ) {
    throw new Error(`${where}: span.col must be an integer >= 1`);
  }
  return { file: rec.file, line: rec.line as number, ...(rec.col !== undefined ? { col: rec.col as number } : {}) };
}

/** The node's own span: an explicit override (factory wrappers) or
 *  the first stack frame outside the package. Mandatory on the wire,
 *  so a span-less runtime is an authoring error, not a silent gap. */
export function nodeSpan(explicit: Span | undefined, what: string): Span {
  if (explicit !== undefined) return asSpan(explicit, what);
  const span = callerSpan();
  if (!span) {
    throw new Error(
      `${what}: could not capture a source span — pass span: { file, line } explicitly`,
    );
  }
  return span;
}

const ARG_FIELDS: Record<string, readonly string[]> = {
  literal: ["kind", "value"],
  artifact: ["kind", "output", "selector"],
  package_command: ["kind", "package", "command"],
  host: ["kind", "path"],
};

export function asArg(v: unknown, where: string): WorkspaceArg {
  const rec = asRecord(v, where);
  const kind = rec.kind;
  if (typeof kind !== "string" || !(kind in ARG_FIELDS) || kind === "host") {
    throw new Error(
      `${where}: kind must be "literal", "artifact" or "package_command"`,
    );
  }
  rejectUnknownFields(where, rec, ARG_FIELDS[kind]!);
  // literal values may be empty (schema has no minLength there);
  // every reference field names something and must be non-empty
  if (kind === "literal") {
    if (typeof rec.value !== "string") throw new Error(`${where}.value must be a string`);
  } else {
    for (const field of ARG_FIELDS[kind]!) {
      if (field === "selector") asSelector(rec[field], `${where}.${field}`);
      else if (field !== "kind") asName(rec[field], `${where}.${field}`);
    }
  }
  return v as WorkspaceArg;
}

export function asPath(v: unknown, where: string): WorkspacePath {
  const rec = asRecord(v, where);
  const kind = rec.kind;
  if (typeof kind !== "string" || !(kind in ARG_FIELDS) || kind === "package_command") {
    throw new Error(`${where}: kind must be "literal", "artifact" or "host"`);
  }
  rejectUnknownFields(where, rec, ARG_FIELDS[kind]!);
  if (kind === "literal") {
    if (typeof rec.value !== "string") throw new Error(`${where}.value must be a string`);
  } else {
    for (const field of ARG_FIELDS[kind]!) {
      if (field === "selector") asSelector(rec[field], `${where}.${field}`);
      else if (field !== "kind") asName(rec[field], `${where}.${field}`);
    }
  }
  return v as WorkspacePath;
}

/** An artifact selector: `.` (the whole artifact) or a normalized
 *  relative POSIX path — no leading slash, no empty, `.` or `..`
 *  segments. Anything else could escape the addressed artifact. */
export function asSelector(v: unknown, where: string): string {
  const s = asName(v, where);
  if (s === ".") return s;
  if (s.includes("\0") || s.startsWith("/") ||
    s.split("/").some((seg) => seg === "" || seg === "." || seg === "..")) {
    throw new Error(
      `${where}: selector must be "." or a normalized relative POSIX path ` +
        `(no NUL, leading "/", empty, "." or ".." segments) — got ${JSON.stringify(s)}`,
    );
  }
  return s;
}

/** A captured repository file is a single normalized relative POSIX path.
 *  `.` denotes a directory, never an individual file; backslashes are
 *  ambiguous across hosts and must not turn into platform separators. */
export function asRepoFilePath(v: unknown, where: string): string {
  const path = asName(v, where);
  if (path.startsWith("/") || path.includes("\\") || path.includes("\0") ||
    path.split("/").some((part) => part === "" || part === "." || part === "..")) {
    throw new Error(
      `${where}: repository file path must be a normalized relative POSIX file path ` +
        `(no leading "/", backslash, NUL, empty, "." or ".." segments) — got ${JSON.stringify(path)}`,
    );
  }
  return path;
}

/** Environment values are DATA — literal text or an artifact
 *  reference. A package_command here would invoke a program from a
 *  value position, which workspace IR never admits (0052 §2.2, core E128):
 *  invoke tools from exec argv or a run_bash interpreter pin. */
export function asEnv(
  v: Record<string, WorkspaceArg> | undefined,
  where: string,
): Record<string, WorkspaceArg> | undefined {
  if (v === undefined) return undefined;
  const rec = asRecord(v, where);
  // Environment order does not change command identity: object and
  // fluent inputs must emit identical bytes regardless of insertion order.
  const entries: [string, WorkspaceArg][] = [];
  for (const k of Object.keys(rec).sort()) {
    const a = asArg(rec[k], `${where}["${k}"]`);
    if (a.kind === "package_command") {
      throw new Error(
        `${where}["${k}"] cannot be a package_command reference; environment values are ` +
          `data (literal or artifact) — invoke package commands from exec argv or a run_bash interpreter pin`,
      );
    }
    entries.push([k, a]);
  }
  return entries.length ? Object.fromEntries(entries) : undefined;
}

export function asNames(v: unknown, where: string): string[] | undefined {
  if (v === undefined) return undefined;
  if (!Array.isArray(v)) throw new Error(`${where} must be an array of output names`);
  const out = v.map((n, i) => asName(n, `${where}[${i}]`));
  return out.length > 0 ? out : undefined;
}


export function asCommand(v: unknown, where: string): WorkspaceCommand {
  const rec = asRecord(v, where);
  if (rec.kind === "exec") {
    rejectUnknownFields(where, rec, ["kind", "span", "argv", "env", "cwd"]);
    asSpan(rec.span, where);
    if (!Array.isArray(rec.argv)) throw new Error(`${where}.argv must be an array`);
    rec.argv.forEach((a, i) => asArg(a, `${where}.argv[${i}]`));
    asEnv(rec.env as Record<string, WorkspaceArg> | undefined, `${where}.env`);
    if (rec.cwd !== undefined) asPath(rec.cwd, `${where}.cwd`);
    return v as WorkspaceCommand;
  }
  if (rec.kind === "run_bash") {
    rejectUnknownFields(where, rec, ["kind", "span", "interpreter", "body", "env", "cwd", "line_map"]);
    asSpan(rec.span, where);
    asArg(rec.interpreter, `${where}.interpreter`);
    if (typeof rec.body !== "string") throw new Error(`${where}.body must be a string`);
    asEnv(rec.env as Record<string, WorkspaceArg> | undefined, `${where}.env`);
    if (rec.cwd !== undefined) asPath(rec.cwd, `${where}.cwd`);
    if (rec.line_map !== undefined) {
      if (
        !Array.isArray(rec.line_map) ||
        rec.line_map.some((l) => !Number.isInteger(l) || (l as number) < 1)
      ) {
        throw new Error(`${where}.line_map must be an array of integers >= 1`);
      }
    }
    return v as WorkspaceCommand;
  }
  throw new Error(`${where}: kind must be "exec" or "run_bash" — use exec()/runBash()`);
}

export function asSource(v: unknown, where: string): WorkspaceSource {
  const rec = asRecord(v, where);
  if (rec.kind === "repo_file") {
    rejectUnknownFields(where, rec, ["kind", "path"]);
    asRepoFilePath(rec.path, `${where}.path`);
    return v as WorkspaceSource;
  }
  if (rec.kind === "artifact_file") {
    rejectUnknownFields(where, rec, ["kind", "output", "selector"]);
    asName(rec.output, `${where}.output`);
    asSelector(rec.selector, `${where}.selector`);
    return v as WorkspaceSource;
  }
  throw new Error(`${where}: kind must be "repo_file" or "artifact_file"`);
}

export function asContent(v: unknown, where: string): WorkspaceContent {
  const rec = asRecord(v, where);
  if (rec.kind === "identity") {
    rejectUnknownFields(where, rec, ["kind"]);
    return v as WorkspaceContent;
  }
  if (rec.kind === "literal") {
    rejectUnknownFields(where, rec, ["kind", "text"]);
    if (typeof rec.text !== "string") throw new Error(`${where}.text must be a string`);
    return v as WorkspaceContent;
  }
  if (rec.kind === "template") {
    rejectUnknownFields(where, rec, ["kind", "template", "variables"]);
    if (typeof rec.template !== "string") throw new Error(`${where}.template must be a string`);
    const vars = asRecord(rec.variables, `${where}.variables`);
    for (const [k, val] of Object.entries(vars)) {
      if (typeof val !== "string") {
        throw new Error(`${where}.variables["${k}"] must be a string`);
      }
    }
    return v as WorkspaceContent;
  }
  throw new Error(`${where}: kind must be "identity", "literal" or "template"`);
}

export function asDestination(v: unknown, where: string): WorkspaceDestination {
  const rec = asRecord(v, where);
  if (rec.kind === "symlink" || rec.kind === "tracked_copy") {
    rejectUnknownFields(where, rec, ["kind", "path"]);
    asDestinationPath(rec.path, `${where}.path`);
    return v as WorkspaceDestination;
  }
  if (rec.kind === "managed_block") {
    rejectUnknownFields(where, rec, ["kind", "path", "marker"]);
    asDestinationPath(rec.path, `${where}.path`);
    asName(rec.marker, `${where}.marker`);
    return v as WorkspaceDestination;
  }
  throw new Error(`${where}: kind must be "symlink", "tracked_copy" or "managed_block"`);
}

/** A destination path must be absolute or `~/`-prefixed with
 *  normalized segments — mirrors the core's E102 rule so an escape
 *  fails at authoring, not only at decoded-IR admission. */
export function asDestinationPath(v: unknown, where: string): string {
  const path = asName(v, where);
  const rest = path.startsWith("~/")
    ? path.slice(2)
    : path.startsWith("/")
      ? path.slice(1)
      : null;
  const normalized = rest !== null
    && rest.length > 0
    && !rest.endsWith("/")
    && !rest.includes("\0")
    && rest.split("/").every((s) => s !== "" && s !== "." && s !== "..");
  if (!normalized) {
    throw new Error(
      `${where}: must be absolute or start with ~/ and use normalized segments ` +
        `(no NUL, ".", "..", empty or trailing segments) — got ${JSON.stringify(path)}`,
    );
  }
  return path;
}

export function asFile(v: unknown, where: string): WorkspaceFile {
  const rec = asRecord(v, where);
  rejectUnknownFields(where, rec, ["span", "source", "content", "destination"]);
  asSpan(rec.span, where);
  const content = asContent(rec.content, `${where}.content`);
  if (rec.source !== undefined) asSource(rec.source, `${where}.source`);
  else if (content.kind !== "literal") {
    throw new Error(
      `${where}: source is required unless content is literalText(...) — ` +
        `a ${content.kind} content has no bytes without an origin`,
    );
  }
  asDestination(rec.destination, `${where}.destination`);
  return v as WorkspaceFile;
}

export function asCalendar(v: unknown, where: string): WorkspaceCalendar {
  const rec = asRecord(v, where);
  const time = (w: string): void => {
    if (typeof rec.time !== "string" || !/^([01][0-9]|2[0-3]):[0-5][0-9]$/.test(rec.time)) {
      throw new Error(`${w}.time must be local "HH:MM" (named timezones and cron are rejected)`);
    }
  };
  if (rec.kind === "daily") {
    rejectUnknownFields(where, rec, ["kind", "time"]);
    time(where);
    return v as WorkspaceCalendar;
  }
  if (rec.kind === "weekly") {
    rejectUnknownFields(where, rec, ["kind", "weekday", "time"]);
    if (!["mon", "tue", "wed", "thu", "fri", "sat", "sun"].includes(rec.weekday as string)) {
      throw new Error(`${where}.weekday must be one of mon..sun`);
    }
    time(where);
    return v as WorkspaceCalendar;
  }
  throw new Error(`${where}: kind must be "daily" or "weekly"`);
}

export function asProducer(v: unknown, where: string): WorkspaceProducer {
  if (typeof v === "string") return { kind: "recipe", recipe: asName(v, where) };
  const rec = asRecord(v, where);
  if (rec.kind === "recipe") {
    rejectUnknownFields(where, rec, ["kind", "recipe"]);
    return { kind: "recipe", recipe: asName(rec.recipe, `${where}.recipe`) };
  }
  if (rec.kind === "provider") {
    rejectUnknownFields(where, rec, ["kind", "provider"]);
    const inner = asRecord(rec.provider, `${where}.provider`);
    rejectUnknownFields(`${where}.provider`, inner, ["fetch", "span"]);
    return {
      kind: "provider",
      provider: {
        fetch: asFetch(inner.fetch, `${where}.provider.fetch`),
        span: asSpan(inner.span, `${where}.provider`),
      },
    };
  }
  throw new Error(`${where}: producer must be a recipe output name or provider(fetch(...))`);
}

export function asFetch(v: unknown, where: string): Fetch {
  const rec = asRecord(v, where);
  if (typeof rec.kind !== "string" || rec.kind === "") {
    throw new Error(`${where} must be a fetch spec (githubRelease()/tarball()/…)`);
  }
  return v as Fetch;
}

/** Deep copy + freeze: the returned value shares no mutable state
 *  with the caller's inputs and can never be mutated afterwards. */
export function freezeDeep<T>(value: T): T {
  if (Array.isArray(value)) {
    return Object.freeze(value.map(freezeDeep)) as unknown as T;
  }
  if (value !== null && typeof value === "object") {
    // fromEntries preserves own "__proto__" keys rather than assigning
    // through Object.prototype's setter and silently dropping an env var.
    const entries = Object.entries(value as Record<string, unknown>)
      .map(([key, entry]) => [key, freezeDeep(entry)] as const);
    return Object.freeze(Object.fromEntries(entries)) as T;
  }
  return value;
}

/** A catalog-name collision carries BOTH declaration spans as labels
 *  (A1-06: terminal and JSON show both sources, like core E125). */
export function duplicateError(name: string, first: Span, again: Span): DiagnosticError {
  return new DiagnosticError({
    code: diagnosticCodes.duplicateWorkspaceOutput,
    severity: "error",
    message:
      `duplicate output '${name}' — output names are the single catalog namespace (0052 §2.1)`,
    labels: [
      { span: first, note: "first declared here" },
      { span: again, note: "also declared here" },
    ],
  });
}
