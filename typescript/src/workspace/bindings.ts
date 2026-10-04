/** Pure declarations: no registry, import-time effect or implicit ownership. */
import { rejectUnknownFields } from "../fields.ts";
import type { TaskContext, WorkspaceAction, WorkspaceInput, WorkspaceInputRef, WorkspaceMutationLock, WorkspaceStep, WorkspaceStepValue } from "./ir.ts";
import { asCommand, asDestinationPath, asName, asRecord, asRepoFilePath, asSelector, asSpan, freezeDeep, nodeSpan } from "./validate.ts";

export function inputFile(name: string, path: string): WorkspaceInput {
  return freezeDeep({ name: asName(name, "inputFile(name)"), span: nodeSpan(undefined, "inputFile(...)"), origin: { kind: "repo_file", path: asRepoFilePath(path, "inputFile(path)") } });
}
export function inputDirectory(name: string, path: string, selection: { include: readonly string[]; exclude?: readonly string[] }): WorkspaceInput {
  const value = { name, span: nodeSpan(undefined, "inputDirectory(...)"), origin: { kind: "repo_directory", path, include: [...selection.include], ...(selection.exclude ? { exclude: [...selection.exclude] } : {}) } };
  return freezeDeep(asInput(value, "inputDirectory(...)"));
}
export function input(name: string | WorkspaceInput): WorkspaceInputRef {
  return freezeDeep({ kind: "input", input: asName(typeof name === "string" ? name : name.name, "input(name)") });
}
export function lock(scope: "user" | "store", key: string): WorkspaceMutationLock {
  return freezeDeep(asLock({ scope, key, span: nodeSpan(undefined, "lock(...)") }, "lock(...)"));
}
export function ensureArtifact(output: string): WorkspaceAction {
  return freezeDeep({ kind: "ensure_artifact", output: asName(output, "ensureArtifact(output)"), span: nodeSpan(undefined, "ensureArtifact(...)") });
}
export function asPatterns(value: unknown, where: string, required: boolean): string[] {
  if (!Array.isArray(value) || required && value.length === 0) throw new Error(`${where}: expected ${required ? "nonempty " : ""}pattern array`);
  return [...new Set(value.map((pattern, index) => asRepoFilePath(pattern, `${where}[${index}]`)))].sort();
}
export function asInput(value: unknown, where: string): WorkspaceInput {
  const record = asRecord(value, where);
  rejectUnknownFields(where, record, ["name", "span", "origin"]);
  const origin = asRecord(record.origin, `${where}.origin`);
  const name = asName(record.name, `${where}.name`);
  const span = asSpan(record.span, where);
  if (origin.kind === "repo_file") {
    rejectUnknownFields(`${where}.origin`, origin, ["kind", "path"]);
    return { name, span, origin: { kind: "repo_file", path: asRepoFilePath(origin.path, `${where}.origin.path`) } };
  } else if (origin.kind === "repo_directory") {
    rejectUnknownFields(`${where}.origin`, origin, ["kind", "path", "include", "exclude"]);
    const path = asSelector(origin.path, `${where}.origin.path`);
    const include = asPatterns(origin.include, `${where}.origin.include`, true);
    const exclude = origin.exclude === undefined ? [] : asPatterns(origin.exclude, `${where}.origin.exclude`, false);
    return { name, span, origin: { kind: "repo_directory", path, include, ...(exclude.length ? { exclude } : {}) } };
  } else throw new Error(`${where}.origin: expected repo_file or repo_directory`);
}
export function asLock(value: unknown, where: string): WorkspaceMutationLock {
  const record = asRecord(value, where);
  rejectUnknownFields(where, record, ["scope", "key", "span"]);
  if (record.scope !== "user" && record.scope !== "store") throw new Error(`${where}.scope: expected user or store`);
  if (typeof record.key !== "string" || !/^[!-~]{1,128}$/.test(record.key)) throw new Error(`${where}.key: expected 1..128 printable non-whitespace ASCII bytes`);
  asSpan(record.span, where);
  return value as WorkspaceMutationLock;
}
export function asTaskContext(value: unknown, where: string): TaskContext {
  const record = asRecord(value, where);
  rejectUnknownFields(where, record, ["kind", "mutable_paths"]);
  if (record.kind !== "host" || !Array.isArray(record.mutable_paths)) throw new Error(`${where}: expected an explicit host context and mutable_paths array`);
  record.mutable_paths.forEach((path, index) => asDestinationPath(path, `${where}.mutable_paths[${index}]`));
  return value as TaskContext;
}
export function asStep(value: WorkspaceStepValue, where: string): WorkspaceStep {
  const record = asRecord(value, where);
  if (record.kind === "exec" || record.kind === "run_bash") return { command: asCommand(value, where) };
  if (record.kind === "ensure_artifact") {
    rejectUnknownFields(where, record, ["kind", "output", "span"]);
    asName(record.output, `${where}.output`); asSpan(record.span, where);
    return { action: value as WorkspaceAction };
  }
  rejectUnknownFields(where, record, ["command", "action"]);
  if (Object.keys(record).length !== 1) throw new Error(`${where}: a local step contains exactly one command or action`);
  if (record.command !== undefined) return { command: asCommand(record.command, where) };
  return asStep(record.action as WorkspaceAction, where);
}
export function lockKey(value: WorkspaceMutationLock): string { return `${value.scope}\0${value.key}`; }
