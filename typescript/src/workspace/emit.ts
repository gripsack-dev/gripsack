/** Current workspace emission: named references, role admission and cycles. */

import type { HostFacts } from "../facts.ts";
export const IR_VERSION = 10;
import type { Span } from "../span.ts"
import type { ProbeRequest } from "../probe.ts";
import type {
  EnvironmentNode,
  PackageNode,
  WorkspaceArg,
  WorkspaceArtifactRef,
  WorkspaceCommand,
  WorkspaceOutputKind,
  WorkspaceOutputNode,
  WorkspacePackageCommand,
  WorkspacePath,
  WorkspaceValue,
  WorkspaceInput,
  WorkspaceStep,
} from "./ir.ts";
import { asName, asRecord, asSelector, asSpan, duplicateError } from "./validate.ts";
import { DiagnosticError, diagnosticCodes, errorAt } from "../diagnostic.ts";
import { checkEnvironmentPrefix, checkPackageTarget, checkTargetAndLayout } from "./target.ts";
import { asInput } from "./bindings.ts";
import { rejectUnknownFields } from "../fields.ts";
import { fileCheckAttachments } from "./file-checks.ts";
import { checkLiteralDestinations } from "./destinations.ts";
import { serviceNames, serviceProperties } from "./services.ts";

/** Catalog roles never conflate publication checks or retention with
 * production closure. Ordered local commands are intra-output and
 * keep their list order; no catalog `ordering` edge is fabricated. */
type EdgeRole = "runtime" | "task_prereq" | "validation" | "retention";

interface Edge {
  from: string;
  to: string;
  span: Span;
  expected: readonly WorkspaceOutputKind[];
  role: EdgeRole;
  targetConsumer?: EnvironmentNode | PackageNode;
}

function isDependency(role: EdgeRole): boolean {
  return role === "runtime" || role === "task_prereq";
}

/** Only packages carry artifacts addressable by artifact references. */
const ARTIFACT_KINDS: readonly WorkspaceOutputKind[] = ["package"];

/** Selector admission at emit: constructors guard their own values,
 *  but hand-built nodes reach the emitter unchecked — reject escapes
 *  here too, labeling the source span (core E130). */
function checkSelector(selector: string, span: Span, site: string): void {
  try {
    asSelector(selector, site);
  } catch (error) {
    throw errorAt(
      diagnosticCodes.invalidWorkspaceValue,
      `workspace: ${(error as Error).message}`,
      span,
      "referenced here",
    );
  }
}

/** Environment values are data — a package_command in a value
 *  position would invoke a program where only bytes are admitted
 *  (0052 §2.2, core E128). */
function envValueError(from: string, key: string, span: Span): DiagnosticError {
  return errorAt(
    diagnosticCodes.badWorkspaceContext,
    `workspace: output '${from}' environment variable '${key}' cannot be a package_command ` +
      `reference; environment values are data (literal or artifact) — invoke package commands ` +
      `from exec argv or a run_bash interpreter pin`,
    span,
    "referenced here",
  );
}

type Subject = "none" | "package" | "file";

function checkSubjectBinding(
  value: WorkspaceArg | WorkspacePath, subject: Subject, cwd: boolean, span: Span, from: string,
): void {
  if (value.kind !== "source" && value.kind !== "output") return;
  if (value.kind === "source" &&
    (subject === "package" || (subject === "file" && !cwd && value.selector === "."))) return;
  const reason = value.kind === "output"
    ? "outputPath requires a writable production output"
    : subject === "file"
    ? 'an attached file supports only sourcePath(".") as data, not as a working directory'
    : "sourcePath requires a package check subject or an explicitly attached file";
  throw errorAt(diagnosticCodes.badWorkspaceContext, `workspace: '${from}': ${reason}`, span, "referenced here");
}

function commandEdges(cmd: WorkspaceCommand, from: string, role: EdgeRole, edges: Edge[], subject: Subject = "none"): void {
  const artifactEdge = (a: WorkspaceArtifactRef): void => {
    checkSelector(a.selector, cmd.span, `output '${from}' artifact reference`);
    edges.push({ from, to: a.output, span: cmd.span, expected: ARTIFACT_KINDS, role });
  };
  const packageEdge = (a: WorkspacePackageCommand): void => {
    edges.push({ from, to: a.package, span: cmd.span, expected: ["package"], role });
  };
  const argEdge = (a: WorkspaceArg | WorkspacePath, cwd = false): void => {
    checkSubjectBinding(a, subject, cwd, cmd.span, from);
    if (a.kind === "artifact") artifactEdge(a);
    else if (a.kind === "package_command") packageEdge(a);
  };
  if (cmd.kind === "exec") {
    for (const arg of cmd.argv) argEdge(arg);
  } else {
    // a run_bash body runs under a pinned package interpreter only —
    // a literal or artifact interpreter is ambient host discovery
    if (cmd.interpreter.kind !== "package_command") {
      throw errorAt(
        diagnosticCodes.badWorkspaceContext,
        `workspace: output '${from}' run_bash interpreter must be a package_command reference ` +
          `pinning the tool through a declared package; a literal or artifact interpreter is ` +
          `ambient host discovery, which workspace IR never admits`,
        cmd.span,
        "referenced here",
      );
    }
    packageEdge(cmd.interpreter);
  }
  for (const [key, a] of Object.entries(cmd.env ?? {})) {
    if (a.kind === "package_command") throw envValueError(from, key, cmd.span);
    argEdge(a);
  }
  if (cmd.cwd) argEdge(cmd.cwd, true);
}

function stepEdges(step: WorkspaceStep, from: string, span: Span, role: EdgeRole, edges: Edge[]): void {
  if ("command" in step) commandEdges(step.command, from, role, edges);
  else edges.push({ from, to: step.action.output, span: step.action.span ?? span, expected: ARTIFACT_KINDS, role });
}

function outputEdges(node: WorkspaceOutputNode, checkSubject: Subject): Edge[] {
  const edges: Edge[] = [];
  const names = (
    list: string[] | undefined,
    expected: readonly WorkspaceOutputKind[],
    role: EdgeRole,
    targetConsumer?: EnvironmentNode | PackageNode,
  ): void => {
    for (const to of list ?? []) {
      const edge: Edge = { from: node.name, to, span: node.span, expected, role };
      if (targetConsumer) edge.targetConsumer = targetConsumer;
      edges.push(edge);
    }
  };
  switch (node.kind) {
    case "package":
      names(node.runtime, ["package"], "runtime", node);
      break;
    case "environment":
      names(node.packages, ["package"], "runtime", node);
      for (const [key, a] of Object.entries(node.env ?? {})) {
        if (a.kind === "package_command") throw envValueError(node.name, key, node.span);
        checkSubjectBinding(a, "none", false, node.span, node.name);
        if (a.kind === "artifact") {
          checkSelector(a.selector, node.span, `environment '${node.name}' variable '${key}'`);
          edges.push({ from: node.name, to: a.output, span: node.span, expected: ARTIFACT_KINDS, role: "runtime" });
        }
      }
      break;
    case "service":
      serviceProperties(node, `service '${node.name}'`);
      names(node.depends_on, ["service"], "runtime");
      break;
    case "task":
      for (const step of node.steps) stepEdges(step, node.name, node.span, "runtime", edges);
      names(node.deps, ["task"], "task_prereq");
      names(serviceNames(node.services, `task '${node.name}' services`), ["service"], "runtime");
      names(node.environment !== undefined ? [node.environment] : undefined, ["environment"], "runtime");
      names(node.checks, ["check"], "validation");
      break;
    case "schedule":
      names([node.task], ["task"], "retention");
      break;
    case "check":
      commandEdges(node.run, node.name, "runtime", edges, checkSubject);
      names([node.subject], ["package", "environment", "service", "task", "schedule", "check", "profile", "hook"], "validation");
      break;
    case "profile":
      for (const f of node.files ?? []) {
        if (f.source?.kind === "artifact_file") {
          checkSelector(f.source.selector, f.span, `profile '${node.name}' file source`);
          edges.push({ from: node.name, to: f.source.output, span: f.span, expected: ARTIFACT_KINDS, role: "runtime" });
        }
        if (f.source?.kind === "tree") {
          edges.push({ from: node.name, to: f.source.output, span: f.span, expected: ARTIFACT_KINDS, role: "runtime" });
        }
        for (const check of f.checks ?? []) {
          edges.push({ from: node.name, to: check.check, span: check.span, expected: ["check"], role: "validation" });
        }
      }
      names(node.environment !== undefined ? [node.environment] : undefined, ["environment"], "retention");
      names(node.schedules, ["schedule"], "retention");
      names(node.hooks, ["hook"], "retention");
      break;
    case "hook":
      commandEdges(node.run, node.name, "runtime", edges);
      break;
  }
  return edges;
}

function orList(kinds: readonly string[]): string {
  return kinds.length === 1
    ? `a ${kinds[0]}`
    : `one of ${kinds.map((k) => `'${k}'`).join(", ")}`;
}

/** Serialize a workspace value as the IR10 envelope —
 *  `{ir_version: 10, host, workspace}`. Admission mirrors the decoded core:
 *  every typed reference is checked against the catalog (unknown names,
 *  wrong output kinds), artifact selectors must be normalized relative
 *  paths, environment values are data only, selected packages and
 *  runtime dependencies agree on OS/arch/ABI with compatible OS floors;
 *  package producers and environment prefixes respect the declared layout,
 *  command bindings require an owning subject, and literal destinations
 *  cannot have conflicting owners. The core independently admits the IR.
 *  Host facts are injected for the envelope only, never used to select
 *  target outputs; the hostname never crosses into the IR. A provisional
 *  empty workspace has no IR (`null`) until its requested probes are bound. */
export function emitWorkspaceIr(
  value: WorkspaceValue,
  facts: HostFacts,
  tags: string[] = [],
  pendingProbes?: readonly ProbeRequest[],
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
  if (typeof ir !== "object" || ir === null || !Array.isArray(ir.outputs)) {
    throw new Error("emitWorkspaceIr: workspace value must carry an outputs array");
  }
  asSpan(ir.span, "emitWorkspaceIr: workspace");
  rejectUnknownFields("emitWorkspaceIr: workspace", ir, ["span", "name", "outputs", "inputs", "mutation_locks"]);
  const inputCatalog = new Map<string, WorkspaceInput>();
  for (const raw of ir.inputs ?? []) {
    const input = asInput(raw, "workspace input");
    const previous = inputCatalog.get(input.name);
    if (previous) throw duplicateError(input.name, previous.span, input.span);
    inputCatalog.set(input.name, input);
  }
  if (ir.outputs.length === 0) {
    if (pendingProbes?.length) return "null";
    throw errorAt(diagnosticCodes.invalidWorkspaceValue,
      "workspace(...): outputs must declare at least one output", ir.span, "in this declaration");
  }

  const catalog = new Map<string, WorkspaceOutputNode>();
  for (const node of ir.outputs) {
    const rec = asRecord(node, "emitWorkspaceIr: output");
    const name = asName(rec.name, "emitWorkspaceIr: output name");
    asSpan(rec.span, `emitWorkspaceIr: output '${name}'`);
    const prev = catalog.get(name);
    if (prev) throw duplicateError(name, prev.span, node.span);
    checkTargetAndLayout(node);
    catalog.set(name, node);
  }
  const attachedChecks = fileCheckAttachments(catalog);
  checkLiteralDestinations(catalog.values());

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
      if (a.kind === "input" && !inputCatalog.has(a.input)) {
        throw errorAt(diagnosticCodes.unknownWorkspaceRef, `workspace: unknown captured input '${a.input}'`, span, "input referenced here");
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
    const checkSubject = node.kind === "check" && catalog.get(node.subject)?.kind === "package"
      ? "package" : attachedChecks.has(node.name) ? "file" : "none";
    for (const e of outputEdges(node, checkSubject)) {
      const target = catalog.get(e.to);
      if (!target) {
        throw errorAt(
          diagnosticCodes.unknownWorkspaceRef,
          `workspace: output '${e.from}' references unknown output '${e.to}'`,
          e.span,
          "referenced here",
        );
      }
      if (!e.expected.includes(target.kind)) {
        throw new DiagnosticError({
          code: diagnosticCodes.unknownWorkspaceRef,
          severity: "error",
          message:
            `workspace: output '${e.from}' expects '${e.to}' to be ${orList(e.expected)} — ` +
            `it is a '${target.kind}'`,
          labels: [
            { span: e.span, note: "referenced here" },
            { span: target.span, note: `'${e.to}' declared here as a '${target.kind}'` },
          ],
        });
      }
      if (target.kind === "package") {
        if (e.targetConsumer) checkPackageTarget(e.targetConsumer, target);
        if (node.kind === "environment") checkEnvironmentPrefix(node, target);
      }
      if (isDependency(e.role)) depEdges.push(e);
    }
    if (node.kind === "profile" && node.launchers !== undefined) {
      const environment = catalog.get(node.environment!);
      if (environment?.kind === "environment") {
        for (const command of node.launchers.commands) {
          const exporters = environment.packages.filter((name) => {
            const selected = catalog.get(name);
            return selected?.kind === "package" && Object.hasOwn(selected.commands, command);
          });
          if (exporters.length !== 1) {
            throw errorAt(diagnosticCodes.unknownWorkspaceRef,
              `profile '${node.name}': launcher '${command}' must have exactly one environment export (found ${exporters.length})`,
              node.span, "launchers declared here");
          }
        }
      }
    }
    if (node.kind === "task") {
      for (const step of node.steps ?? []) if ("command" in step) collectCommandKeys(step.command);
    }
    if (node.kind === "check" || node.kind === "hook") collectCommandKeys(node.run);
    if (node.kind === "environment") collectArgKeys(Object.values(node.env ?? {}), node.span);
  }
  for (const { pkg: target, command, span } of commandKeys) {
    if (!Object.hasOwn(target.commands, command)) {
      const have = Object.keys(target.commands).join(", ");
      throw new DiagnosticError({
        code: diagnosticCodes.unknownWorkspaceRef,
        severity: "error",
        message: `workspace: package '${target.name}' has no command '${command}'`,
        labels: [{ span, note: "referenced here" }],
        ...(have ? { help: `package '${target.name}' provides: ${have}` } : {}),
      });
    }
  }

  // Validation and retention edges may legitimately close a dependency loop.
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
        throw new DiagnosticError({
          code: diagnosticCodes.workspaceCycle,
          severity: "error",
          message: `workspace: dependency cycle ${cycle.join(" -> ")}`,
          labels: cycle.slice(0, -1).map((name) => ({
            span: catalog.get(name)!.span,
            note: `'${name}' declared here`,
          })),
        });
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
      workspace: ir,
    },
    null,
    2,
  );
}
