/** Workspace declarations (0052 A1) — split into cohesive modules
 *  (plan/0052 §3 ~400-line review): ir.ts (wire types), validate.ts
 *  (shared runtime guards), commands.ts (exec/runBash + dedent),
 *  files.ts (origin/content/destination axes), outputs.ts (the nine
 *  output constructors + workspace entrypoint), emit.ts (reference/
 *  selector/target/cycle admission + the v4 envelope).
 *  ../workspace.ts is the supported re-export surface. */

import type { HostFacts } from "../facts.ts";
import { IR_VERSION } from "../graph.ts";
import type { Span } from "../module.ts";
import type {
  PackageNode,
  WorkspaceArg,
  WorkspaceArtifactRef,
  WorkspaceCommand,
  WorkspaceOutputKind,
  WorkspaceOutputNode,
  WorkspacePackageCommand,
  WorkspacePath,
  WorkspacePlatform,
  WorkspaceValue,
} from "./ir.ts";
import { asName, asRecord, asSelector, asSpan, duplicateError, spanAt } from "./validate.ts";

/** Catalog roles never conflate publication checks or retention with
 * production closure. Ordered local commands are intra-output and
 * keep their list order; no catalog `ordering` edge is fabricated. */
type EdgeRole = "production" | "build_input" | "runtime" | "task_prereq" | "validation" | "retention";

interface Edge {
  from: string;
  to: string;
  span: Span;
  expected: readonly WorkspaceOutputKind[];
  role: EdgeRole;
}

function isDependency(role: EdgeRole): boolean {
  return role === "production" || role === "build_input" ||
    role === "runtime" || role === "task_prereq";
}

/** Outputs whose artifacts an `artifact` reference may address —
 *  recipes and packages carry artifacts; images (and every consumer
 *  kind) do not (0052 §2.2). */
const ARTIFACT_KINDS: readonly WorkspaceOutputKind[] = ["recipe", "package"];

/** Selector admission at emit: constructors guard their own values,
 *  but hand-built nodes reach the emitter unchecked — reject escapes
 *  here too, naming the source span (core E130). */
function checkSelector(selector: string, span: Span, site: string): void {
  try {
    asSelector(selector, site);
  } catch (e) {
    throw new Error(`workspace: ${(e as Error).message} (referenced at ${spanAt(span)})`);
  }
}

/** Environment values are data — a package_command in a value
 *  position would invoke a program where only bytes are admitted
 *  (0052 §2.2, core E128). */
function envValueError(from: string, key: string, span: Span): Error {
  return new Error(
    `workspace: output '${from}' environment variable '${key}' cannot be a package_command ` +
      `reference; environment values are data (literal or artifact) — invoke package commands ` +
      `from exec argv or a run_bash interpreter pin (referenced at ${spanAt(span)})`,
  );
}

function commandEdges(cmd: WorkspaceCommand, from: string, role: EdgeRole, edges: Edge[]): void {
  const artifactEdge = (a: WorkspaceArtifactRef): void => {
    checkSelector(a.selector, cmd.span, `output '${from}' artifact reference`);
    edges.push({ from, to: a.output, span: cmd.span, expected: ARTIFACT_KINDS, role });
  };
  const packageEdge = (a: WorkspacePackageCommand): void => {
    edges.push({ from, to: a.package, span: cmd.span, expected: ["package"], role });
  };
  /** Command position (exec argv, run_bash interpreter pin, cwd): the
   *  only places a tool invocation is legal. */
  const argEdge = (a: WorkspaceArg | WorkspacePath): void => {
    if (a.kind === "artifact") artifactEdge(a);
    else if (a.kind === "package_command") packageEdge(a);
  };
  if (cmd.kind === "exec") {
    cmd.argv.forEach(argEdge);
  } else {
    // a run_bash body runs under a pinned package interpreter only —
    // a literal or artifact interpreter is ambient host discovery
    if (cmd.interpreter.kind !== "package_command") {
      throw new Error(
        `workspace: output '${from}' run_bash interpreter must be a package_command reference ` +
          `pinning the tool through a declared package; a literal or artifact interpreter is ` +
          `ambient host discovery, which v4 never admits (referenced at ${spanAt(cmd.span)})`,
      );
    }
    packageEdge(cmd.interpreter);
  }
  for (const [key, a] of Object.entries(cmd.env ?? {})) {
    if (a.kind === "package_command") throw envValueError(from, key, cmd.span);
    argEdge(a);
  }
  if (cmd.cwd) argEdge(cmd.cwd);
}

function outputEdges(node: WorkspaceOutputNode): Edge[] {
  const edges: Edge[] = [];
  const names = (
    list: string[] | undefined,
    expected: readonly WorkspaceOutputKind[],
    role: EdgeRole,
  ): void => {
    for (const to of list ?? []) {
      edges.push({ from: node.name, to, span: node.span, expected, role });
    }
  };
  switch (node.kind) {
    case "recipe":
      for (const c of node.steps ?? []) commandEdges(c, node.name, "build_input", edges);
      names(node.checks, ["check"], "validation");
      break;
    case "package":
      if (node.producer.kind === "recipe") {
        names([node.producer.recipe], ["recipe"], "production");
      }
      names(node.runtime, ["package"], "runtime");
      break;
    case "environment":
      names(node.packages, ["package"], "runtime");
      for (const [key, a] of Object.entries(node.env ?? {})) {
        if (a.kind === "package_command") throw envValueError(node.name, key, node.span);
        if (a.kind === "artifact") {
          checkSelector(a.selector, node.span, `environment '${node.name}' variable '${key}'`);
          edges.push({ from: node.name, to: a.output, span: node.span, expected: ARTIFACT_KINDS, role: "runtime" });
        }
      }
      break;
    case "task":
      commandEdges(node.run, node.name, "runtime", edges);
      names(node.deps, ["task"], "task_prereq");
      names(node.environment !== undefined ? [node.environment] : undefined, ["environment"], "runtime");
      names(node.checks, ["check"], "validation");
      break;
    case "schedule":
      names([node.task], ["task"], "retention");
      break;
    case "check":
      commandEdges(node.run, node.name, "runtime", edges);
      names([node.subject], ["recipe", "package", "environment", "task", "schedule", "check", "image", "profile", "hook"], "validation");
      break;
    case "image":
      names(node.packages, ["package"], "runtime");
      break;
    case "profile":
      for (const f of node.files ?? []) {
        if (f.source?.kind === "artifact_file") {
          checkSelector(f.source.selector, f.span, `profile '${node.name}' file source`);
          edges.push({ from: node.name, to: f.source.output, span: f.span, expected: ARTIFACT_KINDS, role: "runtime" });
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

/** Per-output target identity: exact os/arch/abi/minimum_os equality
 *  between a provider and its consumer — the conservative v4 rule.
 *  The injected host facts are never consulted, so a workspace
 *  targeting another platform is admitted as declared. */
function sameTarget(a: WorkspacePlatform, b: WorkspacePlatform): boolean {
  return a.os === b.os && a.arch === b.arch &&
    (a.abi ?? null) === (b.abi ?? null) &&
    (a.minimum_os ?? null) === (b.minimum_os ?? null);
}

function requireSameTarget(
  relation: string,
  consumer: WorkspaceOutputNode & { target: WorkspacePlatform },
  provider: WorkspaceOutputNode & { target: WorkspacePlatform },
): void {
  if (!sameTarget(consumer.target, provider.target)) {
    throw new Error(
      `workspace: ${relation} — target of '${provider.name}' ` +
        `(${JSON.stringify(provider.target)}) does not equal target of '${consumer.name}' ` +
        `(${JSON.stringify(consumer.target)}); exact os/arch/abi/minimum_os equality is the v4 ` +
        `target identity ('${consumer.name}' declared at ${spanAt(consumer.span)}, ` +
        `'${provider.name}' declared at ${spanAt(provider.span)})`,
    );
  }
}

/** Target and layout admission over resolved references (run after the
 *  reference pass, so every lookup is guaranteed present and typed):
 *  a package's target must equal its producer recipe's, an environment
 *  or image selection must equal the consumer's target, and a
 *  fixed_prefix package cannot enter a selection — the v4 wire has no
 *  consumer prefix slot to install it at. */
function checkTargetsAndLayouts(catalog: Map<string, WorkspaceOutputNode>): void {
  for (const node of catalog.values()) {
    if (node.kind === "package" && node.producer.kind === "recipe") {
      const recipe = catalog.get(node.producer.recipe)!;
      if (recipe.kind === "recipe") {
        requireSameTarget(`package '${node.name}' producer target mismatch`, node, recipe);
      }
    }
    if (node.kind === "environment" || node.kind === "image") {
      for (const name of node.packages) {
        const selected = catalog.get(name)!;
        if (selected.kind !== "package") continue; // the reference pass rejected this
        requireSameTarget(`${node.kind} '${node.name}' package selection target mismatch`, node, selected);
        if (selected.layout === "fixed_prefix") {
          throw new Error(
            `workspace: ${node.kind} '${node.name}' selects package '${selected.name}' with layout ` +
              `'fixed_prefix' but declares no install prefix — the v4 wire has no consumer prefix ` +
              `slot, so fixed_prefix packages cannot be selected yet ` +
              `('${node.name}' declared at ${spanAt(node.span)}, ` +
              `'${selected.name}' declared at ${spanAt(selected.span)})`,
          );
        }
      }
    }
  }
}

function orList(kinds: readonly string[]): string {
  return kinds.length === 1
    ? `a ${kinds[0]}`
    : `one of ${kinds.map((k) => `'${k}'`).join(", ")}`;
}

/** Serialize a workspace value as the v4 workspace IR envelope —
 *  `{ir_version: 4, host, workspace}`, never `modules` (the schema
 *  admits exactly one of the two). Admission mirrors the decoded core:
 *  every typed reference is checked against the catalog (unknown names,
 *  wrong output kinds), artifact selectors must be normalized relative
 *  paths, environment values are data only, per-output targets must
 *  match exactly between producer/consumer, fixed_prefix packages
 *  cannot enter a prefix-less selection, and dependency cycles throw —
 *  all naming the declaration spans. Host facts are core-injected for
 *  the envelope only, never consulted for target admission; the
 *  hostname never crosses into the IR. */
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
      if (isDependency(e.role)) depEdges.push(e);
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

  checkTargetsAndLayouts(catalog);

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
