/** Workspace declarations (0052 A1) — split into cohesive modules
 *  (plan/0052 §3 ~400-line review): ir.ts (wire types), validate.ts
 *  (shared runtime guards), commands.ts (exec/runBash + dedent),
 *  files.ts (origin/content/destination axes), outputs.ts (the nine
 *  output constructors + workspace entrypoint), emit.ts (reference/
 *  cycle admission + the v4 envelope). ../workspace.ts is the
 *  supported re-export surface. */

import type { HostFacts } from "../facts.ts";
import { IR_VERSION } from "../graph.ts";
import type { Span } from "../module.ts";
import type {
  PackageNode,
  WorkspaceArg,
  WorkspaceCommand,
  WorkspaceOutputKind,
  WorkspaceOutputNode,
  WorkspacePath,
  WorkspaceValue,
} from "./ir.ts";
import { asName, asRecord, asSpan, duplicateError, spanAt } from "./validate.ts";

interface Edge {
  from: string;
  to: string;
  span: Span;
  expected: readonly WorkspaceOutputKind[];
  why: string;
  /** Dependency edges feed cycle detection; validation/consumer
   *  edges (checks, subjects, profile wiring) are checked for
   *  existence and kind but can legitimately close a loop. */
  dep: boolean;
}

/** Outputs whose artifacts an `artifact` reference may address. */
const ARTIFACT_KINDS: readonly WorkspaceOutputKind[] = ["recipe", "package", "image"];

function commandEdges(cmd: WorkspaceCommand, from: string, edges: Edge[]): void {
  const argEdge = (a: WorkspaceArg | WorkspacePath): void => {
    if (a.kind === "artifact") {
      edges.push({ from, to: a.output, span: cmd.span, expected: ARTIFACT_KINDS, why: "artifact", dep: true });
    } else if (a.kind === "package_command") {
      edges.push({ from, to: a.package, span: cmd.span, expected: ["package"], why: "package_command", dep: true });
    }
  };
  if (cmd.kind === "exec") {
    cmd.argv.forEach(argEdge);
  } else {
    argEdge(cmd.interpreter);
  }
  for (const a of Object.values(cmd.env ?? {})) argEdge(a);
  if (cmd.cwd) argEdge(cmd.cwd);
}

function outputEdges(node: WorkspaceOutputNode): Edge[] {
  const edges: Edge[] = [];
  const names = (
    list: string[] | undefined,
    expected: readonly WorkspaceOutputKind[],
    why: string,
    dep: boolean,
  ): void => {
    for (const to of list ?? []) {
      edges.push({ from: node.name, to, span: node.span, expected, why, dep });
    }
  };
  switch (node.kind) {
    case "recipe":
      for (const c of node.steps ?? []) commandEdges(c, node.name, edges);
      names(node.checks, ["check"], "checks", false);
      break;
    case "package":
      if (node.producer.kind === "recipe") {
        names([node.producer.recipe], ["recipe"], "producer", true);
      }
      names(node.runtime, ["package"], "runtime", true);
      break;
    case "environment":
      names(node.packages, ["package"], "packages", true);
      for (const a of Object.values(node.env ?? {})) {
        if (a.kind === "artifact") {
          edges.push({ from: node.name, to: a.output, span: node.span, expected: ARTIFACT_KINDS, why: "artifact", dep: true });
        } else if (a.kind === "package_command") {
          edges.push({ from: node.name, to: a.package, span: node.span, expected: ["package"], why: "package_command", dep: true });
        }
      }
      break;
    case "task":
      commandEdges(node.run, node.name, edges);
      names(node.deps, ["task"], "deps", true);
      names(node.environment !== undefined ? [node.environment] : undefined, ["environment"], "environment", true);
      names(node.checks, ["check"], "checks", false);
      break;
    case "schedule":
      names([node.task], ["task"], "task", false);
      break;
    case "check":
      commandEdges(node.run, node.name, edges);
      names([node.subject], ["recipe", "package", "environment", "task", "schedule", "check", "image", "profile", "hook"], "subject", false);
      break;
    case "image":
      names(node.packages, ["package"], "packages", true);
      break;
    case "profile":
      for (const f of node.files ?? []) {
        if (f.source?.kind === "artifact_file") {
          edges.push({ from: node.name, to: f.source.output, span: f.span, expected: ARTIFACT_KINDS, why: "artifact_file", dep: true });
        }
      }
      names(node.environment !== undefined ? [node.environment] : undefined, ["environment"], "environment", false);
      names(node.schedules, ["schedule"], "schedules", false);
      names(node.hooks, ["hook"], "hooks", false);
      break;
    case "hook":
      commandEdges(node.run, node.name, edges);
      break;
  }
  return edges;
}

function orList(kinds: readonly string[]): string {
  return kinds.length === 1
    ? `a ${kinds[0]}`
    : `one of ${kinds.map((k) => `'${k}'`).join(", ")}`;
}

/** Serialize a workspace value as the v4 workspace IR envelope —
 *  `{ir_version: 4, host, workspace}`, never `modules` (the schema
 *  admits exactly one of the two). Every typed reference is checked
 *  against the catalog: unknown names, wrong output kinds and
 *  dependency cycles throw naming the declaration spans. Host facts
 *  are core-injected; the hostname never crosses into the IR. */
export function emitWorkspaceIr(
  value: WorkspaceValue,
  facts: HostFacts,
  tags: string[] = [],
): string {
  if (
    typeof value !== "object" || value === null ||
    (value as WorkspaceValue).__gripsack !== "workspace"
  ) {
    throw new Error(
      `emitWorkspaceIr expects a workspace({...}) value — got ${
        JSON.stringify(value instanceof Promise ? "a promise" : typeof value)
      }`,
    );
  }
  const ir = value.ir;
  if (
    typeof ir !== "object" || ir === null || !Array.isArray(ir.outputs) ||
    ir.outputs.length === 0
  ) {
    throw new Error("emitWorkspaceIr: workspace value must carry at least one output");
  }
  asSpan(ir.span, "emitWorkspaceIr: workspace");

  const catalog = new Map<string, WorkspaceOutputNode>();
  for (const node of ir.outputs) {
    const rec = asRecord(node, "emitWorkspaceIr: output");
    const name = asName(rec.name, "emitWorkspaceIr: output name");
    asSpan(rec.span, `emitWorkspaceIr: output '${name}'`);
    const prev = catalog.get(name);
    if (prev) throw duplicateError(name, prev.span, node.span);
    catalog.set(name, node);
  }

  // typed references: existence + expected kinds
  const depEdges: Edge[] = [];
  const commandKeys: { pkg: PackageNode; command: string; span: Span }[] = [];
  const collectArgKeys = (args: Iterable<WorkspaceArg | WorkspacePath>, span: Span): void => {
    for (const a of args) {
      if (a.kind === "package_command") {
        const target = catalog.get(a.package);
        if (target?.kind === "package") {
          commandKeys.push({ pkg: target, command: a.command, span });
        }
      }
    }
  };
  const collectCommandKeys = (cmd: WorkspaceCommand): void => {
    const args: (WorkspaceArg | WorkspacePath)[] = cmd.kind === "exec"
      ? [...cmd.argv, ...Object.values(cmd.env ?? {})]
      : [cmd.interpreter, ...Object.values(cmd.env ?? {})];
    if (cmd.cwd) args.push(cmd.cwd);
    collectArgKeys(args, cmd.span);
  };
  for (const node of catalog.values()) {
    for (const e of outputEdges(node)) {
      const target = catalog.get(e.to);
      if (!target) {
        throw new Error(
          `workspace: output '${e.from}' references unknown output '${e.to}' ` +
            `(referenced at ${spanAt(e.span)})`,
        );
      }
      if (!e.expected.includes(target.kind)) {
        throw new Error(
          `workspace: output '${e.from}' expects '${e.to}' to be ${orList(e.expected)} — ` +
            `it is a '${target.kind}' (referenced at ${spanAt(e.span)}, ` +
            `declared at ${spanAt(target.span)})`,
        );
      }
      if (e.dep) depEdges.push(e);
    }
    if (node.kind === "recipe") for (const c of node.steps ?? []) collectCommandKeys(c);
    if (node.kind === "task" || node.kind === "check" || node.kind === "hook") {
      collectCommandKeys(node.run);
    }
    if (node.kind === "environment") collectArgKeys(Object.values(node.env ?? {}), node.span);
  }
  for (const { pkg: target, command, span } of commandKeys) {
    if (!(command in target.commands)) {
      const have = Object.keys(target.commands).join(", ");
      throw new Error(
        `workspace: package '${target.name}' has no command '${command}' ` +
          `(referenced at ${spanAt(span)}${have ? `; it provides: ${have}` : ""})`,
      );
    }
  }

  // dependency cycles over production/build edges only — validation
  // edges (a recipe gated by a check on the package it produces) may
  // legitimately close a loop
  const adj = new Map<string, Edge[]>();
  for (const e of depEdges) {
    const list = adj.get(e.from) ?? [];
    list.push(e);
    adj.set(e.from, list);
  }
  const state = new Map<string, "visiting" | "done">();
  const stack: string[] = [];
  const visit = (n: string): void => {
    state.set(n, "visiting");
    stack.push(n);
    for (const e of adj.get(n) ?? []) {
      const s = state.get(e.to);
      if (s === undefined) visit(e.to);
      else if (s === "visiting") {
        const cycle = [...stack.slice(stack.indexOf(e.to)), e.to];
        throw new Error(
          `workspace: dependency cycle ${cycle.join(" -> ")} (` +
            cycle
              .slice(0, -1)
              .map((c) => `'${c}' declared at ${spanAt(catalog.get(c)!.span)}`)
              .join(", ") +
            `)`,
        );
      }
    }
    stack.pop();
    state.set(n, "done");
  };
  for (const n of catalog.keys()) {
    if (!state.has(n)) visit(n);
  }

  return JSON.stringify(
    {
      ir_version: IR_VERSION,
      host: {
        os: facts.os,
        arch: facts.arch,
        tags,
        ...(facts.libc !== null ? { libc: facts.libc } : {}),
      },
      workspace: { span: ir.span, outputs: ir.outputs },
    },
    null,
    2,
  );
}
