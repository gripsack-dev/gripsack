/** v7 named-output constructors: pure, frozen values with source spans. */

import { rejectUnknownFields } from "../fields.ts";
import type { Fetch } from "../fetch.ts";
import { callerSpan } from "../module.ts";
import type { Span } from "../module.ts";
import type {
  CheckNode,
  CheckSpec,
  CondaEnvironmentSpec,
  CondaEnvironmentSource,
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
  PixiLockSpec,
  PixiLockSource,
  WorkspaceMutationLock,
  WorkspaceCalendar,
  WorkspaceFn,
  WorkspaceOutput,
  WorkspaceOutputNode,
  WorkspacePlatform,
  WorkspaceProducer,
  WorkspaceSpec,
  WorkspaceValue,
  AcquisitionSource,
  WorkspaceWeekday,
} from "./ir.ts";
import {
  asCalendar,
  asCommand,
  asEnv,
  asFile,
  asHostRuntime,
  asSystemRequirements,
  asName,
  asNames,
  asProducer,
  asAcquisitionSource,
  asRecord,
  duplicateError,
  freezeDeep,
  nodeSpan,
} from "./validate.ts";
import { asExecution, asInstallPrefix, asLayout, asPlatform } from "./target.ts";
import { asInput, asLock, asStep, asTaskContext, lockKey } from "./bindings.ts";
import { imageProperties } from "./image.ts";


/** A per-output build/target platform. */
export function targetPlatform(spec: WorkspacePlatform): WorkspacePlatform {
  return freezeDeep(asPlatform(spec, "targetPlatform(...)"));
}

/** Lower an authoring source — a bare `Fetch`, a plain
 *  CondaEnvironmentSpec/PixiLockSpec, or a pre-built
 *  `condaEnvironment(...)`/`pixiFromLock(...)` value — to the current tagged
 *  wire shape, stamping `span` on freshly wrapped variants. */
function asSourceValue(source: unknown, span: Span, where: string): AcquisitionSource {
  const rec = asRecord(source, where);
  if (rec.kind === "fetch" || rec.kind === "conda_environment" || rec.kind === "pixi_lock") {
    return asAcquisitionSource(source, where);
  }
  if (typeof rec.kind === "string" && rec.kind !== "") {
    // a bare Fetch spec — the v6 fetch wrapper carries the provenance span
    return asAcquisitionSource({ kind: "fetch", fetch: source, span }, where);
  }
  if (rec.channels !== undefined || rec.packages !== undefined) {
    rejectUnknownFields(where, rec, ["channels", "packages", "platforms", "systemRequirements"]);
    const conda = source as CondaEnvironmentSpec;
    return asAcquisitionSource({
      kind: "conda_environment",
      channels: conda.channels,
      packages: conda.packages,
      ...(conda.platforms ? { platforms: conda.platforms } : {}),
      ...(conda.systemRequirements !== undefined
        ? { system_requirements: asSystemRequirements(conda.systemRequirements, `${where}.systemRequirements`) }
        : {}),
      span,
    }, where);
  }
  const pixiLock = source as PixiLockSpec;
  rejectUnknownFields(where, rec, ["manifest", "lock", "environment"]);
  return asAcquisitionSource({
    kind: "pixi_lock",
    manifest: pixiLock.manifest,
    lock: pixiLock.lock,
    environment: pixiLock.environment,
    span,
  }, where);
}

/** A coherent binary Conda environment source, solved through Rattler —
 *  usable directly as `recipe({ source })` or inside `provider(...)`. */
export function condaEnvironment(spec: CondaEnvironmentSpec): CondaEnvironmentSource {
  const span = callerSpan();
  if (!span) {
    throw new Error(
      "condaEnvironment(...): could not capture a source span — construct the source explicitly",
    );
  }
  const rec = asRecord(spec, "condaEnvironment(...)");
  rejectUnknownFields("condaEnvironment(...)", rec, ["channels", "packages", "platforms", "systemRequirements"]);
  return freezeDeep(asSourceValue(spec, span, "condaEnvironment(...)") as CondaEnvironmentSource);
}

/** An explicit Pixi lock import: `manifest`/`lock` are workspace input
 *  names, `environment` selects the Pixi environment from the lock. */
export function pixiFromLock(spec: PixiLockSpec): PixiLockSource {
  const span = callerSpan();
  if (!span) {
    throw new Error(
      "pixiFromLock(...): could not capture a source span — construct the source explicitly",
    );
  }
  const rec = asRecord(spec, "pixiFromLock(...)");
  rejectUnknownFields("pixiFromLock(...)", rec, ["manifest", "lock", "environment"]);
  return freezeDeep(asSourceValue(spec, span, "pixiFromLock(...)") as PixiLockSource);
}

/** A direct provider-backed acquisition for `pkg({ producer })` —
 *  the package resolves through the declared provider with its own
 *  provenance, no synthetic recipe output. Accepts a `Fetch` (wrapped as
 *  `{"kind":"fetch",…}`) or a `condaEnvironment(...)`/`pixiFromLock(...)`
 *  source value. */
export function provider(source: Fetch | AcquisitionSource): WorkspaceProducer {
  const span = callerSpan();
  if (!span) {
    throw new Error(
      "provider(...): could not capture a source span — construct the producer explicitly",
    );
  }
  return freezeDeep({ kind: "provider", provider: asSourceValue(source, span, "provider(...)") });
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
  const source = asSourceValue(spec.source, span, `${what}: source`);
  const target = asPlatform(spec.target, `${what}: target`);
  if (source.kind === "conda_environment" && source.system_requirements !== undefined && target.os !== "linux") {
    throw new Error(`${what}: Conda systemRequirements require a Linux target`);
  }
  if (spec.output_kind !== "file" && spec.output_kind !== "tree") {
    throw new Error(`${what}: output_kind must be "file" or "tree"`);
  }
  const steps = spec.steps?.length
    ? spec.steps.map((c, i) => asStep(c, `${what}: steps[${i}]`))
    : undefined;
  const checks = asNames(spec.checks, `${what}: checks`);
  const node: RecipeNode = {
    name,
    span,
    kind: "recipe",
    source,
    execution: asExecution(spec.execution, `${what}: execution`),
    output_kind: spec.output_kind,
    target,
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
    "producer", "commands", "runtime", "hostRuntime", "target", "layout", "span",
  ]);
  const span = nodeSpan(spec.span, what);
  const producer = asProducer(spec.producer, `${what}: producer`);
  const commandsRec = asRecord(spec.commands, `${what}: commands`);
  const commands: Record<string, string> = Object.create(null);
  for (const [k, v] of Object.entries(commandsRec)) {
    commands[k] = asName(v, `${what}: commands["${k}"]`);
  }
  const runtime = asNames(spec.runtime, `${what}: runtime`);
  const target = asPlatform(spec.target, `${what}: target`);
  if (producer.kind === "provider" && producer.provider.kind === "conda_environment" &&
    producer.provider.system_requirements !== undefined && target.os !== "linux") {
    throw new Error(`${what}: Conda systemRequirements require a Linux target`);
  }
  const hostRuntime = spec.hostRuntime === undefined
    ? undefined
    : asHostRuntime(spec.hostRuntime, `${what}: hostRuntime`);
  if (hostRuntime !== undefined && (target.os !== "linux" || target.abi !== "gnu")) {
    throw new Error(`${what}: hostRuntime requires an explicit Linux GNU target`);
  }
  const node: PackageNode = {
    name,
    span,
    kind: "package",
    producer,
    commands,
    ...(runtime ? { runtime } : {}),
    ...(hostRuntime !== undefined ? { host_runtime: hostRuntime } : {}),
    target,
    layout: asLayout(spec.layout, `${what}: layout`),
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
  rejectUnknownFields(what, spec, ["packages", "target", "prefix", "env", "span"]);
  const span = nodeSpan(spec.span, what);
  const packages = asNames(spec.packages, `${what}: packages`) ?? [];
  const env = asEnv(spec.env, `${what}: env`);
  const node: EnvironmentNode = {
    name,
    span,
    kind: "environment",
    packages,
    target: asPlatform(spec.target, `${what}: target`),
    ...(spec.prefix !== undefined ? { prefix: asInstallPrefix(spec.prefix, `${what}: prefix`) } : {}),
    ...(env ? { env } : {}),
  };
  return makeOutput(node);
}

/** A manual task: ordered local actions, unordered prerequisites and
 * per-invocation postconditions. It never becomes cached production. */
export function task(name: string, spec: TaskSpec): WorkspaceOutput<TaskNode> {
  const what = `task("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["steps", "context", "locks", "deps", "environment", "checks", "span"]);
  const span = nodeSpan(spec.span, what);
  if (!Array.isArray(spec.steps) || spec.steps.length === 0) throw new Error(`${what}: steps must be nonempty`);
  const steps = spec.steps.map((step, index) => asStep(step, `${what}: steps[${index}]`));
  const context = asTaskContext(spec.context ?? { kind: "host", mutable_paths: [] }, `${what}: context`);
  if (spec.locks !== undefined && !Array.isArray(spec.locks)) throw new Error(`${what}: locks must be an array`);
  const locks = spec.locks?.map((lock, index) => asLock(lock, `${what}: locks[${index}]`));
  const deps = asNames(spec.deps, `${what}: deps`);
  const envName = spec.environment !== undefined
    ? asName(spec.environment, `${what}: environment`)
    : undefined;
  const checks = asNames(spec.checks, `${what}: checks`);
  const node: TaskNode = {
    name,
    span,
    kind: "task",
    steps,
    context,
    ...(locks?.length ? { mutation_locks: locks } : {}),
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

/** A runtime package composition exported as an OCI image. */
export function image(name: string, spec: ImageSpec): WorkspaceOutput<ImageNode> {
  const what = `image("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["packages", "target", "base", "destinations", "config", "span"]);
  const span = nodeSpan(spec.span, what);
  const node: ImageNode = {
    name,
    span,
    kind: "image",
    ...imageProperties(spec, what),
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
  rejectUnknownFields(what, spec, ["outputs", "inputs", "name", "span"]);
  if (!Array.isArray(spec.outputs)) throw new Error(`${what}: outputs must be an array`);
  const span = nodeSpan(spec.span, what);
  const name = spec.name === undefined ? undefined : asName(spec.name, `${what}: name`);
  if (name !== undefined && /[\u0000-\u001f\u007f-\u009f]/.test(name)) throw new Error(`${what}: name cannot contain controls`);
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
  outputs.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
  if (spec.inputs !== undefined && !Array.isArray(spec.inputs)) throw new Error(`${what}: inputs must be an array`);
  const inputs = spec.inputs?.map((input, index) => asInput(input, `${what}: inputs[${index}]`)) ?? [];
  const inputNames = new Map<string, Span>();
  for (const input of inputs) {
    const previous = inputNames.get(input.name);
    if (previous) throw duplicateError(input.name, previous, input.span);
    inputNames.set(input.name, input.span);
  }
  inputs.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
  const locks = new Map<string, WorkspaceMutationLock>();
  for (const output of outputs) {
    if (output.kind !== "task") continue;
    for (const lock of output.mutation_locks ?? []) {
      const key = lockKey(lock);
      const previous = locks.get(key);
      if (!previous || `${lock.span.file}:${lock.span.line}:${lock.span.col ?? 0}` <
        `${previous.span.file}:${previous.span.line}:${previous.span.col ?? 0}`) locks.set(key, lock);
    }
  }
  const mutation_locks = [...locks].sort(([left], [right]) => left < right ? -1 : left > right ? 1 : 0).map(([, lock]) => lock);
  return Object.freeze({
    __gripsack: "workspace",
    ir: freezeDeep({ span, outputs, ...(name === undefined ? {} : { name }), ...(inputs.length ? { inputs } : {}), ...(mutation_locks.length ? { mutation_locks } : {}) }),
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
