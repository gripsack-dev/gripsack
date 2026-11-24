/** v10 per-output platform and prefix admission.
 *  Declared producer requirements are checked against consumers, not
 *  against the machine evaluating the workspace. */

import { rejectUnknownFields } from "../fields.ts";
import type {
  EnvironmentNode,
  PackageLayout,
  PackageNode,
  WorkspaceOsVersion,
  WorkspaceOutputNode,
  WorkspacePlatform,
} from "./ir.ts";
import { DiagnosticError, diagnosticCodes, errorAt } from "../diagnostic.ts";
import { asRecord } from "./validate.ts";

function asVersion(value: unknown, where: string): WorkspaceOsVersion {
  const version = asRecord(value, where);
  rejectUnknownFields(where, version, ["major", "minor", "patch"]);
  for (const field of ["major", "minor", "patch"] as const) {
    const part = version[field];
    if (part === undefined && field === "patch") continue;
    if (!Number.isInteger(part) || (part as number) < 0 || (part as number) > 65535) {
      throw new Error(`${where}.${field} must be an integer in 0..65535`);
    }
  }
  return value as WorkspaceOsVersion;
}

export function asPlatform(value: unknown, where: string): WorkspacePlatform {
  const target = asRecord(value, where);
  rejectUnknownFields(where, target, ["os", "arch", "abi", "minimum_os"]);
  if (target.os !== "linux") {
    throw new Error(`${where}.os must be "linux"`);
  }
  if (target.arch !== "x86_64") {
    throw new Error(`${where}.arch must be "x86_64"`);
  }
  if (target.abi !== undefined) {
    const allowed = ["gnu", "musl"];
    if (!allowed.includes(target.abi as string)) {
      throw new Error(`${where}.abi is incompatible with ${target.os}; expected ${allowed.join(" or ")}`);
    }
  }
  if (target.minimum_os !== undefined) asVersion(target.minimum_os, `${where}.minimum_os`);
  return value as WorkspacePlatform;
}

export function asInstallPrefix(value: unknown, where: string): string {
  if (typeof value !== "string" || value.length <= 1 || !value.startsWith("/") ||
    value.includes("\0") ||
    value.slice(1).split("/").some((segment) => segment === "" || segment === "." || segment === "..")) {
    throw new Error(`${where} must be a normalized absolute POSIX install path below /`);
  }
  return value;
}

export function asLayout(value: unknown, where: string): PackageLayout {
  const layout = asRecord(value, where);
  if (layout.kind === "relocatable") {
    rejectUnknownFields(where, layout, ["kind"]);
  } else if (layout.kind === "fixed_prefix") {
    rejectUnknownFields(where, layout, ["kind", "prefix"]);
    asInstallPrefix(layout.prefix, `${where}.prefix`);
  } else if (layout.kind === "prefix_materialized") {
    rejectUnknownFields(where, layout, ["kind"]);
  } else {
    throw new Error(`${where}.kind must be "relocatable", "fixed_prefix" or "prefix_materialized"`);
  }
  return value as PackageLayout;
}

function floorAtMost(provider: WorkspaceOsVersion | undefined, consumer: WorkspaceOsVersion | undefined): boolean {
  if (!provider) return true;
  if (!consumer) return false;
  if (provider.major !== consumer.major) return provider.major < consumer.major;
  if (provider.minor !== consumer.minor) return provider.minor < consumer.minor;
  return (provider.patch ?? 0) <= (consumer.patch ?? 0);
}

/** Selected packages and runtime dependencies promise to run on this target.
 * An artifact referenced only as environment data makes no such promise. */
export function checkPackageTarget(
  consumer: EnvironmentNode | PackageNode,
  provider: PackageNode,
): void {
  const supplied = provider.target;
  const requested = consumer.target;
  if (supplied.os !== requested.os || supplied.arch !== requested.arch ||
    // An environment that omits the ABI is unspecified, not musl: it admits
    // ABI-specific members. A consumer that names an ABI requires it exactly.
    ((requested.abi ?? null) !== null && (supplied.abi ?? null) !== requested.abi) ||
    !floorAtMost(supplied.minimum_os, requested.minimum_os)) {
    throw new DiagnosticError({
      code: diagnosticCodes.unknownWorkspaceRef,
      severity: "error",
      message:
        `workspace: ${consumer.kind} '${consumer.name}' — target of '${provider.name}' (${JSON.stringify(supplied)}) ` +
        `cannot satisfy target of '${consumer.name}' (${JSON.stringify(requested)}): ` +
        `OS/arch must match, a declared ABI must match and provider minimum OS must not exceed the consumer`,
      labels: [
        { span: consumer.span, note: `consumer '${consumer.name}' declared here` },
        { span: provider.span, note: `provider '${provider.name}' declared here` },
      ],
    });
  }
}

/** IR10 applies placement rules to every environment/package reference,
 * including a passive artifact path in an environment variable. */
export function checkEnvironmentPrefix(consumer: EnvironmentNode, provider: PackageNode): void {
  const layout = provider.layout;
  const mismatch = layout.kind === "fixed_prefix" && consumer.prefix !== layout.prefix
    ? `requires the matching install prefix '${layout.prefix}'`
    : layout.kind === "prefix_materialized" && consumer.prefix !== undefined
    ? "derives its install prefix from the frozen lock; an environment prefix cannot override it"
    : undefined;
  if (mismatch) {
    throw new DiagnosticError({
      code: diagnosticCodes.unknownWorkspaceRef,
      severity: "error",
      message: `workspace: environment '${consumer.name}' references package '${provider.name}' ` +
        `with layout ${layout.kind}, which ${mismatch}`,
      labels: [
        { span: consumer.span, note: `environment '${consumer.name}' declared here` },
        { span: provider.span, note: `package '${provider.name}' declared here` },
      ],
    });
  }
}

/** Validate target and layout declarations before comparing resolved edges. */
export function checkTargetAndLayout(node: WorkspaceOutputNode): void {
  try {
    if ("target" in node) {
      asPlatform(node.target, `${node.kind} '${node.name}' target`);
    }
    if (node.kind === "package") {
      const layout = asLayout(node.layout, `package '${node.name}' layout`);
      const source = node.producer.provider.kind;
      if (source === "conda_environment" || source === "pixi_lock") {
        if (layout.kind !== "prefix_materialized") {
          throw new Error(`package '${node.name}': ${source} requires the prefix_materialized layout`);
        }
      } else if (layout.kind === "prefix_materialized" && source !== "package") {
        throw new Error(`package '${node.name}': prefix_materialized requires a Conda/Pixi or imported package`);
      }
    }
    if (node.kind === "environment" && node.prefix !== undefined) {
      asInstallPrefix(node.prefix, `environment '${node.name}' prefix`);
    }
  } catch (error) {
    throw errorAt(diagnosticCodes.invalidWorkspaceValue, (error as Error).message, node.span, "declared here");
  }
}
