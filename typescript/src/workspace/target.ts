/** v5 per-output execution, platform and prefix admission (0052 A1-02).
 *  Declared producer requirements are checked against consumers, not
 *  against the machine evaluating the workspace. */

import { rejectUnknownFields } from "../fields.ts";
import type {
  PackageLayout,
  RecipeExecution,
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
  if (target.os !== "linux" && target.os !== "macos") {
    throw new Error(`${where}.os must be "linux" or "macos"`);
  }
  if (target.arch !== "x86_64" && target.arch !== "aarch64") {
    throw new Error(`${where}.arch must be "x86_64" or "aarch64"`);
  }
  if (target.abi !== undefined) {
    const allowed = target.os === "linux" ? ["gnu", "musl"] : ["darwin"];
    if (!allowed.includes(target.abi as string)) {
      throw new Error(`${where}.abi is incompatible with ${target.os}; expected ${allowed.join(" or ")}`);
    }
  }
  if (target.minimum_os !== undefined) asVersion(target.minimum_os, `${where}.minimum_os`);
  return value as WorkspacePlatform;
}

export function asExecution(value: unknown, where: string): RecipeExecution {
  const execution = asRecord(value, where);
  if (execution.kind === "host") {
    rejectUnknownFields(where, execution, ["kind", "access"]);
    if (execution.access !== "unconfined") {
      throw new Error(`${where}.access must explicitly be "unconfined" (host filesystem/kernel/network access)`);
    }
  } else if (execution.kind === "isolated_linux") {
    rejectUnknownFields(where, execution, ["kind", "worker"]);
    if (execution.worker !== "buildkit") throw new Error(`${where}.worker must be "buildkit"`);
  } else {
    throw new Error(`${where}.kind must be "host" or "isolated_linux"; native acquisition uses a provider-backed package`);
  }
  return value as RecipeExecution;
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
  } else {
    throw new Error(`${where}.kind must be "relocatable" or "fixed_prefix"`);
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

function requireCompatibleTarget(
  relation: string,
  consumer: WorkspaceOutputNode & { target: WorkspacePlatform },
  provider: WorkspaceOutputNode & { target: WorkspacePlatform },
): void {
  const supplied = provider.target;
  const requested = consumer.target;
  if (supplied.os !== requested.os || supplied.arch !== requested.arch ||
    (supplied.abi ?? null) !== (requested.abi ?? null) ||
    !floorAtMost(supplied.minimum_os, requested.minimum_os)) {
    throw new DiagnosticError({
      code: diagnosticCodes.unknownWorkspaceRef,
      severity: "error",
      message:
        `workspace: ${relation} — target of '${provider.name}' (${JSON.stringify(supplied)}) ` +
        `cannot satisfy target of '${consumer.name}' (${JSON.stringify(requested)}): ` +
        `OS/arch/ABI must match and provider minimum OS must not exceed the consumer`,
      labels: [
        { span: consumer.span, note: `consumer '${consumer.name}' declared here` },
        { span: provider.span, note: `provider '${provider.name}' declared here` },
      ],
    });
  }
}

/** Resolved catalog references have already passed kind/existence
 * checks. Hand-built nodes still pass every runtime shape guard here. */
export function checkTargetsAndLayouts(catalog: Map<string, WorkspaceOutputNode>): void {
  const guard = (node: WorkspaceOutputNode, run: () => void): void => {
    try {
      run();
    } catch (error) {
      throw errorAt(
        diagnosticCodes.invalidWorkspaceValue,
        (error as Error).message,
        node.span,
        "declared here",
      );
    }
  };
  for (const node of catalog.values()) {
    if ("target" in node) {
      guard(node, () => void asPlatform(node.target, `${node.kind} '${node.name}' target`));
    }
    if (node.kind === "recipe") {
      guard(node, () => void asExecution(node.execution, `recipe '${node.name}' execution`));
    }
    if (node.kind === "package") {
      guard(node, () => void asLayout(node.layout, `package '${node.name}' layout`));
      if (node.producer.kind === "recipe") {
        const recipe = catalog.get(node.producer.recipe)!;
        if (recipe.kind === "recipe") {
          requireCompatibleTarget(`package '${node.name}' producer target mismatch`, node, recipe);
        }
      }
    }
    if (node.kind === "environment" && node.prefix !== undefined) {
      guard(node, () => void asInstallPrefix(node.prefix, `environment '${node.name}' prefix`));
    }
    if (node.kind === "environment" || node.kind === "image") {
      for (const name of node.packages) {
        const selected = catalog.get(name)!;
        if (selected.kind !== "package") continue; // reference pass rejected this
        requireCompatibleTarget(`${node.kind} '${node.name}' package selection target mismatch`, node, selected);
        if (selected.layout.kind === "fixed_prefix" &&
          (node.kind === "image" || node.prefix !== selected.layout.prefix)) {
          throw new DiagnosticError({
            code: diagnosticCodes.unknownWorkspaceRef,
            severity: "error",
            message:
              `workspace: ${node.kind} '${node.name}' selects package '${selected.name}' with ` +
              `layout fixed_prefix at '${selected.layout.prefix}' but declares no matching ` +
              `install prefix (image prefix materialization is unavailable until B4)`,
            labels: [
              { span: node.span, note: `'${node.name}' declared here` },
              { span: selected.span, note: `'${selected.name}' declared here` },
            ],
          });
        }
      }
    }
  }
}
