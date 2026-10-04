import type { ImageConfig, ImageDestination, ImageNode, ImageOwner, ImageSpec, PackageNode, WorkspaceOutputNode } from "./ir.ts";
import { DiagnosticError, diagnosticCodes } from "../diagnostic.ts";
import { rejectUnknownFields } from "../fields.ts";
import { asArg, asName, asNames, asRecord } from "./validate.ts";
import { asImageReference, asInstallPrefix, asPlatform } from "./target.ts";

type ImageProperties = Pick<ImageNode, "packages" | "target" | "base" | "destinations" | "config">;

export function imageProperties(spec: ImageSpec, where: string): ImageProperties {
  const packages = asNames(spec.packages, `${where}: packages`) ?? [];
  const target = asPlatform(spec.target, `${where}: target`);
  if (target.os !== "linux") throw new Error(`${where}: OCI production requires a Linux target`);
  const destinations: Record<string, ImageDestination> = Object.create(null);
  for (const [name, raw] of Object.entries(asRecord(spec.destinations ?? {}, `${where}: destinations`))) {
    asName(name, `${where}: destination package`);
    const value = asRecord(raw, `${where}: destination ${name}`);
    rejectUnknownFields(`${where}: destination ${name}`, value, ["path", "owner"]);
    destinations[name] = {
      path: asInstallPrefix(value.path, `${where}: destination ${name}`),
      owner: owner(value.owner, `${where}: destination ${name} owner`),
    };
  }
  const config = asRecord(spec.config ?? {}, `${where}: config`);
  rejectUnknownFields(`${where}: config`, config, ["entrypoint", "args", "env", "cwd", "user"]);
  if (config.entrypoint !== undefined && !Array.isArray(config.entrypoint)) throw new Error(`${where}: entrypoint must be an argv array`);
  const entrypoint: NonNullable<ImageConfig["entrypoint"]> = [];
  for (const [index, raw] of ((config.entrypoint ?? []) as unknown[]).entries()) {
    const argument = asArg(raw, `${where}: entrypoint[${index}]`);
    if (argument.kind !== "literal" && argument.kind !== "package_command") throw new Error(`${where}: image arguments cannot refer to host or production paths`);
    if (argument.kind === "literal" && index === 0) absolutePath(argument.value, `${where}: entrypoint executable`);
    if (argument.kind === "package_command" && !packages.includes(argument.package)) throw new Error(`${where}: image command package must be selected explicitly`);
    entrypoint.push(argument);
  }
  const env: Record<string, string> = Object.create(null);
  for (const [key, value] of Object.entries(asRecord(config.env ?? {}, `${where}: environment`))) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key) || typeof value !== "string" || value.includes("\0")) throw new Error(`${where}: invalid image environment binding ${key}`);
    env[key] = value;
  }
  if (config.args !== undefined && !Array.isArray(config.args)) throw new Error(`${where}: image args must be an array`);
  const args = ((config.args ?? []) as unknown[]).map((value) => {
    if (typeof value !== "string" || value.includes("\0")) throw new Error(`${where}: image args must be NUL-free strings`);
    return value;
  });
  return {
    packages, target,
    ...(spec.base === undefined ? {} : { base: asImageReference(spec.base, `${where}: base`) }),
    ...(Object.keys(destinations).length ? { destinations } : {}),
    config: { entrypoint, args, env, cwd: absolutePath(config.cwd ?? "/", `${where}: cwd`), user: owner(config.user, `${where}: user`) },
  };
}

function owner(value: unknown, where: string): ImageOwner {
  if (value === undefined) return { uid: 0, gid: 0 };
  const record = asRecord(value, where);
  rejectUnknownFields(where, record, ["uid", "gid"]);
  for (const key of ["uid", "gid"] as const) {
    if (!Number.isInteger(record[key]) || (record[key] as number) < 0 || (record[key] as number) > 0xffffffff) throw new Error(`${where}.${key} must be an unsigned 32-bit integer`);
  }
  return { uid: record.uid as number, gid: record.gid as number };
}
function absolutePath(value: unknown, where: string): string {
  return value === "/" ? "/" : asInstallPrefix(value, where);
}

/** Destination authority follows runtime packages only, never build tools. */
export function checkImageSelection(image: ImageNode, catalog: Map<string, WorkspaceOutputNode>): void {
  const pending = [...image.packages];
  const selected = new Set<string>();
  const placements: Record<string, PackageNode> = Object.create(null);
  while (pending.length) {
    const name = pending.pop()!;
    if (selected.has(name)) continue;
    selected.add(name);
    const value = catalog.get(name);
    if (value?.kind !== "package") continue; // reference admission diagnoses this
    pending.push(...(value.runtime ?? []));
    const prefix = image.destinations?.[name]?.path ?? `/opt/gripsack/${name}`;
    asInstallPrefix(prefix, `image '${image.name}' destination for '${name}'`);
    if (value.layout.kind === "fixed_prefix" && image.destinations?.[name]?.path !== value.layout.prefix) {
      throw new DiagnosticError({ code: diagnosticCodes.badWorkspaceContext, severity: "error", message: "fixed-prefix image package requires its exact destination", labels: [{ span: image.span, note: "image selection" }, { span: value.span, note: "fixed-prefix package" }] });
    }
    for (const [otherPrefix, other] of Object.entries(placements)) {
      if (prefix === otherPrefix || prefix.startsWith(otherPrefix + "/") || otherPrefix.startsWith(prefix + "/")) {
        throw new DiagnosticError({ code: diagnosticCodes.badWorkspaceContext, severity: "error", message: `image package destinations overlap: ${prefix} and ${otherPrefix}`, labels: [{ span: image.span, note: "image selection" }, { span: value.span, note: "package placement" }, { span: other.span, note: "conflicting package placement" }] });
      }
    }
    placements[prefix] = value;
  }
  for (const name of Object.keys(image.destinations ?? {})) {
    if (!selected.has(name)) throw new DiagnosticError({ code: diagnosticCodes.unknownWorkspaceRef, severity: "error", message: `image destination names unselected runtime package '${name}'`, labels: [{ span: image.span, note: "image selection" }] });
  }
}
