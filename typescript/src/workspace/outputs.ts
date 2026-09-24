/** Workspace declarations (0052 A1) — split into cohesive modules
 *  (plan/0052 §3 ~400-line review): ir.ts (wire types), validate.ts
 *  (shared runtime guards), commands.ts (exec/runBash + dedent),
 *  files.ts (origin/content/destination axes), outputs.ts (the nine
 *  output constructors + workspace entrypoint), emit.ts (reference/
 *  cycle admission + the v4 envelope). ../workspace.ts is the
 *  supported re-export surface. */

import { rejectUnknownFields } from "../fields.ts";
import type { Fetch } from "../fetch.ts";
import { callerSpan } from "../module.ts";
import type { Span } from "../module.ts";
import type {
  CheckNode,
  CheckSpec,
  EnvironmentNode,
  EnvironmentSpec,
  HookNode,
  HookSpec,
  ImageNode,
  ImageSpec,
  PackageNode,
  PackageSpec,
  ProfileNode,
  ProfileSpec,
  RecipeNode,
  RecipeSpec,
  ScheduleNode,
  ScheduleSpec,
  TaskNode,
  TaskSpec,
  WorkspaceCalendar,
  WorkspaceFn,
  WorkspaceOutput,
  WorkspaceOutputNode,
  WorkspacePlatform,
  WorkspaceProducer,
  WorkspaceSpec,
  WorkspaceValue,
  WorkspaceWeekday,
} from "./ir.ts";
import {
  asCalendar,
  asCommand,
  asEnv,
  asFetch,
  asFile,
  asName,
  asNames,
  asPlatform,
  asProducer,
  asRecord,
  duplicateError,
  freezeDeep,
  nodeSpan,
} from "./validate.ts";


/** A per-output build/target platform. */
export function targetPlatform(spec: WorkspacePlatform): WorkspacePlatform {
  asRecord(spec, "targetPlatform(...)");
  rejectUnknownFields("targetPlatform(...)", spec, ["os", "arch", "abi", "minimum_os"]);
  return freezeDeep(asPlatform(spec, "targetPlatform(...)"));
}

/** A direct provider-backed acquisition for `pkg({ producer })` —
 *  the package resolves through the declared provider with its own
 *  provenance, no synthetic recipe output. */
export function provider(fetch: Fetch): WorkspaceProducer {
  const span = callerSpan();
  if (!span) {
    throw new Error(
      "provider(...): could not capture a source span — construct the producer explicitly",
    );
  }
  return freezeDeep({ kind: "provider", provider: { fetch: asFetch(fetch, "provider(...)"), span } });
}


/** Every day at local `HH:MM`. */
export function daily(time: string): WorkspaceCalendar {
  return freezeDeep(asCalendar({ kind: "daily", time }, "daily(...)"));
}

/** Every `<weekday>` at local `HH:MM`. */
export function weekly(weekday: WorkspaceWeekday, time: string): WorkspaceCalendar {
  return freezeDeep(asCalendar({ kind: "weekly", weekday, time }, "weekly(...)"));
}

/** Brand + deep-freeze a constructed node — all nine output
 *  constructors share this exact behavior. */
function makeOutput<N extends WorkspaceOutputNode>(node: N): WorkspaceOutput<N> {
  return Object.freeze({ __gripsack: "workspace_output", ir: freezeDeep(node) });
}

/** A recipe: an ordered local command list over a fetched source,
 *  producing a file or tree artifact for one target platform. */
export function recipe(name: string, spec: RecipeSpec): WorkspaceOutput<RecipeNode> {
  const what = `recipe("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, [
    "source", "execution", "output_kind", "target", "steps", "checks", "span",
  ]);
  const span = nodeSpan(spec.span, what);
  const fetch = asFetch(spec.source, `${what}: source`);
  if (!["native", "host", "isolated_linux"].includes(spec.execution)) {
    throw new Error(`${what}: execution must be "native", "host" or "isolated_linux"`);
  }
  if (spec.output_kind !== "file" && spec.output_kind !== "tree") {
    throw new Error(`${what}: output_kind must be "file" or "tree"`);
  }
  const steps = spec.steps?.length
    ? spec.steps.map((c, i) => asCommand(c, `${what}: steps[${i}]`))
    : undefined;
  const checks = asNames(spec.checks, `${what}: checks`);
  const node: RecipeNode = {
    name,
    span,
    kind: "recipe",
    source: { fetch, span },
    execution: spec.execution,
    output_kind: spec.output_kind,
    target: asPlatform(spec.target, `${what}: target`),
    ...(steps ? { steps } : {}),
    ...(checks ? { checks } : {}),
  };
  return makeOutput(node);
}

/** A package: named commands over a recipe's artifact, with an
 *  explicit runtime closure and a layout contract. (`pkg` because
 *  `package` is a reserved word.) */
export function pkg(name: string, spec: PackageSpec): WorkspaceOutput<PackageNode> {
  const what = `pkg("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, [
    "producer", "commands", "runtime", "target", "layout", "span",
  ]);
  const span = nodeSpan(spec.span, what);
  const producer = asProducer(spec.producer, `${what}: producer`);
  const commandsRec = asRecord(spec.commands, `${what}: commands`);
  const commands: Record<string, string> = {};
  for (const [k, v] of Object.entries(commandsRec)) {
    commands[k] = asName(v, `${what}: commands["${k}"]`);
  }
  const runtime = asNames(spec.runtime, `${what}: runtime`);
  if (spec.layout !== "relocatable" && spec.layout !== "fixed_prefix") {
    throw new Error(`${what}: layout must be "relocatable" or "fixed_prefix"`);
  }
  const node: PackageNode = {
    name,
    span,
    kind: "package",
    producer,
    commands,
    ...(runtime ? { runtime } : {}),
    target: asPlatform(spec.target, `${what}: target`),
    layout: spec.layout,
  };
  return makeOutput(node);
}

/** A process-scoped selection of packages — deploys no personal
 *  profile. */
export function environment(
  name: string,
  spec: EnvironmentSpec,
): WorkspaceOutput<EnvironmentNode> {
  const what = `environment("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["packages", "target", "env", "span"]);
  const span = nodeSpan(spec.span, what);
  const packages = asNames(spec.packages, `${what}: packages`) ?? [];
  const env = asEnv(spec.env, `${what}: env`);
  const node: EnvironmentNode = {
    name,
    span,
    kind: "environment",
    packages,
    target: asPlatform(spec.target, `${what}: target`),
    ...(env ? { env } : {}),
  };
  return makeOutput(node);
}

/** A manual task: one command plus unordered prerequisites and
 *  per-invocation postconditions. */
export function task(name: string, spec: TaskSpec): WorkspaceOutput<TaskNode> {
  const what = `task("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["run", "deps", "environment", "checks", "span"]);
  const span = nodeSpan(spec.span, what);
  const run = asCommand(spec.run, `${what}: run`);
  const deps = asNames(spec.deps, `${what}: deps`);
  const envName = spec.environment !== undefined
    ? asName(spec.environment, `${what}: environment`)
    : undefined;
  const checks = asNames(spec.checks, `${what}: checks`);
  const node: TaskNode = {
    name,
    span,
    kind: "task",
    run,
    ...(deps ? { deps } : {}),
    ...(envName !== undefined ? { environment: envName } : {}),
    ...(checks ? { checks } : {}),
  };
  return makeOutput(node);
}

/** An inert schedule declaration over local daily/weekly time —
 *  registration is E3's and is rejected with an explicit diagnostic
 *  until then; it never activates anything by itself. */
export function schedule(name: string, spec: ScheduleSpec): WorkspaceOutput<ScheduleNode> {
  const what = `schedule("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["task", "trigger", "span"]);
  const span = nodeSpan(spec.span, what);
  const node: ScheduleNode = {
    name,
    span,
    kind: "schedule",
    task: asName(spec.task, `${what}: task`),
    trigger: asCalendar(spec.trigger, `${what}: trigger`),
    scope: "user",
  };
  return makeOutput(node);
}

/** A check over a typed subject — a command that may have effects
 *  and is never run by a read-only preview. */
export function check(name: string, spec: CheckSpec): WorkspaceOutput<CheckNode> {
  const what = `check("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["run", "subject", "span"]);
  const span = nodeSpan(spec.span, what);
  const node: CheckNode = {
    name,
    span,
    kind: "check",
    run: asCommand(spec.run, `${what}: run`),
    subject: asName(spec.subject, `${what}: subject`),
  };
  return makeOutput(node);
}

/** An image: a package set realized for one target platform. */
export function image(name: string, spec: ImageSpec): WorkspaceOutput<ImageNode> {
  const what = `image("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["packages", "target", "span"]);
  const span = nodeSpan(spec.span, what);
  const node: ImageNode = {
    name,
    span,
    kind: "image",
    packages: asNames(spec.packages, `${what}: packages`) ?? [],
    target: asPlatform(spec.target, `${what}: target`),
  };
  return makeOutput(node);
}

/** A profile: files, an environment selection, schedules and hooks —
 *  generations/ownership/journal stay authoritative in the core. */
export function profile(name: string, spec: ProfileSpec): WorkspaceOutput<ProfileNode> {
  const what = `profile("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["files", "environment", "schedules", "hooks", "span"]);
  const span = nodeSpan(spec.span, what);
  const files = spec.files?.length
    ? spec.files.map((f, i) => asFile(f, `${what}: files[${i}]`))
    : undefined;
  const envName = spec.environment !== undefined
    ? asName(spec.environment, `${what}: environment`)
    : undefined;
  const schedules = asNames(spec.schedules, `${what}: schedules`);
  const hooks = asNames(spec.hooks, `${what}: hooks`);
  const node: ProfileNode = {
    name,
    span,
    kind: "profile",
    ...(files ? { files } : {}),
    ...(envName !== undefined ? { environment: envName } : {}),
    ...(schedules ? { schedules } : {}),
    ...(hooks ? { hooks } : {}),
  };
  return makeOutput(node);
}

/** A lifecycle hook — `post_link`/`post_activate`/`on_remove`
 *  semantics preserved from the live Trigger enum. */
export function hook(name: string, spec: HookSpec): WorkspaceOutput<HookNode> {
  const what = `hook("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["run", "trigger", "span"]);
  const span = nodeSpan(spec.span, what);
  if (!["post_link", "post_activate", "on_remove"].includes(spec.trigger)) {
    throw new Error(`${what}: trigger must be "post_link", "post_activate" or "on_remove"`);
  }
  const node: HookNode = {
    name,
    span,
    kind: "hook",
    run: asCommand(spec.run, `${what}: run`),
    trigger: spec.trigger,
  };
  return makeOutput(node);
}

/** Collect returned outputs into a workspace value. Falsy entries
 *  drop out — `ctx.facts.os === "linux" && steam` is the conditional
 *  style, same as `defineEnv` modules. Duplicate names are an
 *  authoring error naming BOTH declaration spans (A1-06). */
export function workspace(spec: WorkspaceSpec): WorkspaceValue {
  const what = "workspace(...)";
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["outputs", "span"]);
  if (!Array.isArray(spec.outputs)) throw new Error(`${what}: outputs must be an array`);
  const span = nodeSpan(spec.span, what);
  const outputs: WorkspaceOutputNode[] = [];
  const seen = new Map<string, Span>();
  for (const o of spec.outputs) {
    if (!o) continue;
    if (typeof o !== "object" || o.__gripsack !== "workspace_output") {
      throw new Error(
        `workspace(...): outputs entries must be recipe()/pkg()/environment()/task()/` +
          `schedule()/check()/image()/profile()/hook() values — got ${
            JSON.stringify(Array.isArray(o) ? "an array" : typeof o)
          }`,
      );
    }
    const prev = seen.get(o.ir.name);
    if (prev) throw duplicateError(o.ir.name, prev, o.ir.span);
    seen.set(o.ir.name, o.ir.span);
    outputs.push(o.ir);
  }
  if (outputs.length === 0) {
    throw new Error(`${what}: outputs must declare at least one output`);
  }
  return Object.freeze({
    __gripsack: "workspace",
    ir: freezeDeep({ span, outputs }),
  });
}

/**
 * Declare the workspace entrypoint:
 *
 * ```ts
 * // gripsack.ts
 * import { defineWorkspace, workspace, pkg, recipe } from "@gripsack/core";
 *
 * export default defineWorkspace((ctx) => workspace({
 *   outputs: [tools, toolsBin],
 * }));
 * ```
 *
 * The function runs inside the sandboxed eval with the core-injected
 * context and must return the workspace synchronously.
 */
export function defineWorkspace(fn: WorkspaceFn): WorkspaceFn {
  return fn;
}
