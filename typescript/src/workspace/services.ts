/** Pure managed-service declarations; Docker lifecycle and trust remain core-owned. */
import { rejectUnknownFields } from "../fields.ts";
import type { ServiceNode, ServiceSpec, WorkspaceOutput } from "./ir.ts";
import { asName, asNames, asRecord, freezeDeep, nodeSpan } from "./validate.ts";
import { asInstallPrefix } from "./target.ts";

const MAX_BINDINGS = 128;
const MAX_ARGUMENTS = 1024;
const MAX_VALUE_BYTES = 65536;
const utf8 = new TextEncoder();

export function serviceNames(value: unknown, where: string): string[] {
  const names = asNames(value, where) ?? [];
  if (names.length > MAX_BINDINGS) throw new Error(`${where} exceeds ${MAX_BINDINGS} entries`);
  return names;
}

function positiveInteger(value: unknown, maximum: number, where: string): void {
  if (!Number.isInteger(value) || (value as number) < 1 || (value as number) > maximum) {
    throw new Error(`${where} must be an integer in 1..${maximum}`);
  }
}
function strings(value: unknown, where: string): asserts value is string[] {
  if (!Array.isArray(value) || value.length > MAX_ARGUMENTS || value.some((part) => typeof part !== "string" || part.includes("\0") || utf8.encode(part).length > MAX_VALUE_BYTES)) {
    throw new Error(`${where} must have at most ${MAX_ARGUMENTS} NUL-free arguments of at most ${MAX_VALUE_BYTES} bytes each`);
  }
}
function loopback(value: unknown): boolean {
  if (value === "::1") return true;
  if (typeof value !== "string") return false;
  const parts = value.split(".");
  return parts.length === 4 && parts[0] === "127" && parts.every((part) =>
    /^(0|[1-9][0-9]{0,2})$/.test(part) && Number(part) <= 255);
}

/** Validate constructor values and hand-built wire nodes at the same boundary. */
export function serviceProperties(spec: ServiceSpec, where: string): Omit<ServiceNode, "name" | "span" | "kind"> {
  if (typeof spec.image !== "string" || !/^[a-z0-9][a-z0-9._:/-]*@sha256:[a-f0-9]{64}$/.test(spec.image) || spec.image.includes("//")) {
    throw new Error(`${where}: image must be an immutable name@sha256 OCI reference`);
  }
  const imageName = spec.image.slice(0, spec.image.indexOf("@"));
  if (imageName.length > 255 || imageName.endsWith("/") || imageName.endsWith(":")) {
    throw new Error(`${where}: image name must be at most 255 characters and cannot end with / or :`);
  }
  const ports = spec.ports === undefined ? [] : spec.ports;
  if (!Array.isArray(ports) || ports.length > MAX_BINDINGS) throw new Error(`${where}: ports must be an array of at most ${MAX_BINDINGS} entries`);
  const bindings = new Set<string>();
  for (const port of ports) {
    const record = asRecord(port, `${where}: port`);
    rejectUnknownFields(`${where}: port`, record, ["host", "container", "address"]);
    positiveInteger(port.host, 65535, `${where}: host port`);
    positiveInteger(port.container, 65535, `${where}: container port`);
    if (!loopback(port.address)) throw new Error(`${where}: port address must be canonical loopback`);
    const binding = `${port.address}:${port.host}`;
    if (bindings.has(binding)) throw new Error(`${where}: duplicate host port binding`);
    bindings.add(binding);
  }
  const volumes = spec.volumes === undefined ? [] : spec.volumes;
  if (!Array.isArray(volumes) || volumes.length > MAX_BINDINGS) throw new Error(`${where}: volumes must be an array of at most ${MAX_BINDINGS} entries`);
  const names = new Set<string>();
  const targets = new Set<string>();
  for (const volume of volumes) {
    const record = asRecord(volume, `${where}: volume`);
    rejectUnknownFields(`${where}: volume`, record, ["name", "target"]);
    if (typeof volume.name !== "string" || volume.name.length > 255 || !/^[a-zA-Z0-9][a-zA-Z0-9_.-]*$/.test(volume.name)) {
      throw new Error(`${where}: volume name must be a named volume, not a host path`);
    }
    asInstallPrefix(volume.target, `${where}: volume target`);
    if (names.has(volume.name) || targets.has(volume.target)) throw new Error(`${where}: duplicate volume name or target`);
    names.add(volume.name);
    targets.add(volume.target);
  }
  const env = spec.env === undefined ? {} : spec.env;
  asRecord(env, `${where}: env`);
  if (Object.keys(env).length > MAX_BINDINGS) throw new Error(`${where}: too many environment entries`);
  for (const [name, binding] of Object.entries(env)) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`${where}: invalid environment name`);
    const record = asRecord(binding, `${where}: env.${name}`);
    if (binding.kind === "literal") {
      rejectUnknownFields(`${where}: env.${name}`, record, ["kind", "value"]);
      if (typeof binding.value !== "string" || binding.value.includes("\0") || utf8.encode(binding.value).length > MAX_VALUE_BYTES) throw new Error(`${where}: environment literal must be NUL-free and at most ${MAX_VALUE_BYTES} bytes`);
    } else if (binding.kind === "inherited") {
      rejectUnknownFields(`${where}: env.${name}`, record, ["kind", "name"]);
      if (typeof binding.name !== "string" || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(binding.name)) throw new Error(`${where}: invalid inherited environment name`);
    } else throw new Error(`${where}: environment binding must be literal or inherited`);
  }
  const command = spec.command === undefined ? [] : spec.command;
  strings(command, `${where}: command`);
  const health = spec.health;
  const healthRecord = asRecord(health, `${where}: health`);
  rejectUnknownFields(`${where}: health`, healthRecord, ["kind", "argv", "interval_seconds", "timeout_seconds", "retries"]);
  if (health.kind !== "command") throw new Error(`${where}: health kind must be command`);
  strings(health.argv, `${where}: health argv`);
  if (!health.argv.length || !health.argv[0]) throw new Error(`${where}: health argv must name an executable`);
  for (const field of ["interval_seconds", "timeout_seconds", "retries"] as const) {
    positiveInteger(health[field], field === "retries" ? 100 : 3600, `${where}: health.${field}`);
  }
  const depends_on = serviceNames(spec.depends_on, `${where}: depends_on`);
  return { image: spec.image, ports, volumes, env, command, health, depends_on };
}

export function service(name: string, spec: ServiceSpec): WorkspaceOutput<ServiceNode> {
  const where = `service("${name}")`;
  asName(name, `${where}: name`);
  const record = asRecord(spec, where);
  rejectUnknownFields(where, record, ["image", "ports", "volumes", "env", "command", "health", "depends_on", "span"]);
  const span = nodeSpan(spec.span, where);
  return Object.freeze({ __gripsack: "workspace_output", ir: freezeDeep({
    kind: "service", name, span, ...serviceProperties(spec, where),
  } as ServiceNode) });
}
