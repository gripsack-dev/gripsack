/** Runtime structural guards for current workspace authoring values. */

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
  WorkspaceSourceV6,
} from "./ir.ts";
import { asPlatform } from "./target.ts";

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
  package_command: ["kind", "package", "command", "sha256"],
  input: ["kind", "input"],
  source: ["kind", "selector"],
  output: ["kind", "selector"],
  host: ["kind", "path"],
};

export function asArg(v: unknown, where: string): WorkspaceArg {
  const rec = asRecord(v, where);
  const kind = rec.kind;
  if (typeof kind !== "string" || !Object.hasOwn(ARG_FIELDS, kind) || kind === "host") {
    throw new Error(
      `${where}: expected a literal, artifact, package_command, input, source or output argument`,
    );
  }
  rejectUnknownFields(where, rec, ARG_FIELDS[kind]!);
  // literal values may be empty (schema has no minLength there);
  // every reference field names something and must be non-empty
  if (kind === "literal") {
    if (typeof rec.value !== "string") throw new Error(`${where}.value must be a string`);
    if (rec.value.includes("\0")) throw new Error(`${where}.value cannot contain NUL`);
  } else {
    for (const field of ARG_FIELDS[kind]!) {
      if (field === "selector") asSelector(rec[field], `${where}.${field}`);
      else if (field === "sha256") { if (rec[field] !== undefined) asSha256(rec[field], `${where}.${field}`); }
      else if (field !== "kind") asName(rec[field], `${where}.${field}`);
    }
  }
  return v as WorkspaceArg;
}

export function asPath(v: unknown, where: string): WorkspacePath {
  const rec = asRecord(v, where);
  const kind = rec.kind;
  if (typeof kind !== "string" || !Object.hasOwn(ARG_FIELDS, kind) || kind === "package_command" || kind === "input") {
    throw new Error(`${where}: expected a literal, artifact, host, source or output path`);
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
  if (s.includes("\0") || s.includes("\\") || s.startsWith("/") ||
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

export function asSha256(value: unknown, where: string): string {
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/.test(value)) throw new Error(`${where}: expected 64 lowercase SHA-256 hex characters`);
  return value;
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
    if (k.length === 0 || k.includes("=") || k.includes("\0")) {
      throw new Error(`${where}: environment names must be nonempty and contain neither '=' nor NUL`);
    }
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
    if (!Array.isArray(rec.argv) || rec.argv.length === 0) throw new Error(`${where}.argv must contain an executable argument`);
    const argv = rec.argv.map((a, i) => asArg(a, `${where}.argv[${i}]`));
    const env = asEnv(rec.env as Record<string, WorkspaceArg> | undefined, `${where}.env`);
    return {
      kind: "exec",
      span: asSpan(rec.span, where),
      argv,
      ...(env ? { env } : {}),
      ...(rec.cwd !== undefined ? { cwd: asPath(rec.cwd, `${where}.cwd`) } : {}),
    };
  }
  if (rec.kind === "run_bash") {
    rejectUnknownFields(where, rec, ["kind", "span", "interpreter", "options", "body", "env", "cwd", "line_map"]);
    asSpan(rec.span, where);
    const interpreter = asArg(rec.interpreter, `${where}.interpreter`);
    if (interpreter.kind !== "package_command") {
      throw new Error(`${where}.interpreter must reference a declared package command`);
    }
    if (!Array.isArray(rec.options) || rec.options.length !== 4 ||
      rec.options.some((value, index) => value !== ["-e", "-u", "-o", "pipefail"][index])) {
      throw new Error(`${where}.options must be -e -u -o pipefail`);
    }
    if (typeof rec.body !== "string") throw new Error(`${where}.body must be a string`);
    const env = asEnv(rec.env as Record<string, WorkspaceArg> | undefined, `${where}.env`);
    if (rec.line_map !== undefined) {
      if (
        !Array.isArray(rec.line_map) ||
        rec.line_map.some((l) => !Number.isInteger(l) || (l as number) < 1)
      ) {
        throw new Error(`${where}.line_map must be an array of integers >= 1`);
      }
    }
    return {
      kind: "run_bash",
      span: asSpan(rec.span, where),
      interpreter,
      options: ["-e", "-u", "-o", "pipefail"],
      body: rec.body,
      ...(env ? { env } : {}),
      ...(rec.cwd !== undefined ? { cwd: asPath(rec.cwd, `${where}.cwd`) } : {}),
      ...(rec.line_map !== undefined ? { line_map: rec.line_map as number[] } : {}),
    };
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
  if (rec.kind === "tree") {
    rejectUnknownFields(where, rec, ["kind", "output", "include", "exclude"]);
    asName(rec.output, `${where}.output`);
    if (!Array.isArray(rec.include) || rec.include.length === 0) throw new Error(`${where}.include must be nonempty`);
    rec.include.forEach((pattern, index) => asRepoFilePath(pattern, `${where}.include[${index}]`));
    if (rec.exclude !== undefined) {
      if (!Array.isArray(rec.exclude)) throw new Error(`${where}.exclude must be an array`);
      rec.exclude.forEach((pattern, index) => asRepoFilePath(pattern, `${where}.exclude[${index}]`));
    }
    return v as WorkspaceSource;
  }
  throw new Error(`${where}: kind must be "repo_file", "artifact_file" or "tree"`);
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
    rejectUnknownFields(where, rec, ["kind", "template", "variables", "result_digest"]);
    if (typeof rec.template !== "string") throw new Error(`${where}.template must be a string`);
    if (rec.result_digest !== undefined) asSha256(rec.result_digest, `${where}.result_digest`);
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
  rejectUnknownFields(where, rec, ["span", "source", "content", "destination", "checks"]);
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
  if (rec.checks !== undefined) {
    if (!Array.isArray(rec.checks)) throw new Error(`${where}.checks must be an array`);
    rec.checks.forEach((value, index) => {
      const check = asRecord(value, `${where}.checks[${index}]`);
      rejectUnknownFields(`${where}.checks[${index}]`, check, ["check", "subject", "stage", "span"]);
      asName(check.check, `${where}.checks[${index}].check`);
      asSpan(check.span, `${where}.checks[${index}]`);
      if (!["source", "rendered", "deployed"].includes(check.subject as string) ||
        !["pre_flip", "post_link", "post_activate"].includes(check.stage as string)) {
        throw new Error(`${where}.checks[${index}] has an invalid subject/stage`);
      }
    });
  }
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
    return {
      kind: "provider",
      provider: asSourceV6(rec.provider, `${where}.provider`),
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

/** A v6 acquisition source — the tagged union behind `RecipeNode.source`
 *  and the provider branch of {@link WorkspaceProducer}. The legacy bare
 *  `{fetch,span}` shape and the brew/pixi fetch kinds are rejected. */
export function asSourceV6(v: unknown, where: string): WorkspaceSourceV6 {
  const rec = asRecord(v, where);
  if (rec.kind === "fetch") {
    rejectUnknownFields(where, rec, ["kind", "fetch", "span"]);
    const fetch = asFetch(rec.fetch, `${where}.fetch`);
    if (fetch.kind === "brew" || fetch.kind === "pixi") {
      throw new Error(
        `${where}.fetch: "${fetch.kind}" fetches are superseded by conda_environment/pixi_lock sources in v6`,
      );
    }
    asSpan(rec.span, where);
    return v as WorkspaceSourceV6;
  }
  if (rec.kind === "conda_environment") {
    rejectUnknownFields(where, rec, ["kind", "channels", "packages", "platforms", "span"]);
    const channels = rec.channels;
    if (!Array.isArray(channels) || channels.length === 0 ||
      channels.some((c) => typeof c !== "string" || c === "")) {
      throw new Error(`${where}.channels must be a non-empty array of channel names/URLs`);
    }
    const packages = asRecord(rec.packages, `${where}.packages`);
    const entries = Object.entries(packages);
    if (entries.length === 0) {
      throw new Error(`${where}.packages must declare at least one package`);
    }
    for (const [name, spec] of entries) {
      if (!/^[a-z0-9][a-z0-9._-]*$/.test(name)) {
        throw new Error(`${where}.packages: "${name}" must be a lowercase Conda package name`);
      }
      if (typeof spec !== "string" || spec === "") {
        throw new Error(`${where}.packages["${name}"] must be a non-empty MatchSpec string ("*" allowed)`);
      }
    }
    if (rec.platforms !== undefined) {
      if (!Array.isArray(rec.platforms)) throw new Error(`${where}.platforms must be an array`);
      rec.platforms.forEach((p, i) => asPlatform(p, `${where}.platforms[${i}]`));
    }
    asSpan(rec.span, where);
    return v as WorkspaceSourceV6;
  }
  if (rec.kind === "pixi_lock") {
    rejectUnknownFields(where, rec, ["kind", "manifest", "lock", "environment", "span"]);
    asName(rec.manifest, `${where}.manifest`);
    asName(rec.lock, `${where}.lock`);
    asName(rec.environment, `${where}.environment`);
    asSpan(rec.span, where);
    return v as WorkspaceSourceV6;
  }
  throw new Error(`${where}: source kind must be "fetch", "conda_environment" or "pixi_lock"`);
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
