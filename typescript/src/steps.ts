/**
 * Steps — the phase building blocks inside a module (0007).
 *
 * Most modules never write steps: the declarative fields are expanded
 * into the conventional pipeline by the core. Declare steps explicitly
 * for control — ordering, resource locks, retries, or a custom action.
 * `steps` and the declarative fields are mutually exclusive per module
 * (E103).
 */

import type { Dest } from "./entries.js";
import type { Fetch } from "./fetch.js";
import { validateResourceRefs } from "./resources.js";
import type { Verify } from "./verify.js";

export type Phase = "fetch" | "build" | "install" | "config" | "verify" | "activate" | "custom";

export type StepAction =
  | { kind: "fetch"; fetch: Fetch }
  | { kind: "build"; spec: Record<string, unknown> }
  | { kind: "install"; entries: unknown[] }
  | { kind: "config_deploy"; entries: unknown[] }
  | { kind: "run"; argv: string[]; env?: Record<string, string>; cwd?: string; outputs?: string[] }
  | { kind: "custom_shell"; script: string; outputs?: string[] };

export interface Step {
  id: string;
  action: StepAction;
  needs?: string[];
  resources?: string[];
  phase?: Phase;
  verify?: Verify;
  retries?: number;
}

export interface StepOpts {
  needs?: string[];
  resources?: string[];
  phase?: Phase;
  verify?: Verify;
  retries?: number;
}

export function step(id: string, action: StepAction, opts?: StepOpts): Step {
  validateResourceRefs(opts?.resources ?? [], `step '${id}'`);
  return { id, action, ...opts };
}

/** Primitives auto-declare their contention domain in the core
 *  (pixi → `pixi-lock`, …) — `resources` is for your own shared
 *  state (0007 §4). Fetch steps retry by default. */
export function fetchStep(fetch: Fetch, id = "fetch", opts?: StepOpts): Step {
  validateResourceRefs(opts?.resources ?? [], `step '${id}'`);
  return { id, action: { kind: "fetch", fetch }, phase: "fetch", ...opts };
}

export function buildStep(
  spec: Record<string, unknown>,
  id = "build",
  opts?: StepOpts,
): Step {
  return { id, action: { kind: "build", spec }, phase: "build", ...opts };
}

/** Deploy built artifacts to their destinations. */
export function installStep(
  entries: Record<string, Dest>,
  id = "install",
  opts?: StepOpts,
): Step {
  return {
    id,
    action: {
      kind: "install",
      entries: Object.entries(entries).map(([from, d]) => ({
        from,
        to: d.to,
        mode: d.mode,
        ...(d.vars ? { vars: d.vars } : {}),
        ...(d.marker !== undefined ? { marker: d.marker } : {}),
      })),
    } as StepAction,
    phase: "install",
    ...opts,
  };
}

/** Deploy config files per their ownership modes (0001 §3.7). */
export function configStep(
  entries: Record<string, Dest>,
  id = "config",
  opts?: StepOpts,
): Step {
  return {
    id,
    action: {
      kind: "config_deploy",
      entries: Object.entries(entries).map(([from, d]) => ({
        from,
        to: d.to,
        mode: d.mode,
        ...(d.vars ? { vars: d.vars } : {}),
        ...(d.marker !== undefined ? { marker: d.marker } : {}),
      })),
    } as StepAction,
    phase: "config",
    ...opts,
  };
}

/** A structured action — the rung between primitives and shell
 *  (0007 §3): argv/env/cwd as data, no shell interpretation, declared
 *  `outputs` make it cacheable (0008 §4). */
export function runStep(
  argv: string[],
  id = "run",
  opts?: StepOpts & {
    env?: Record<string, string>;
    cwd?: string;
    outputs?: string[];
  },
): Step {
  const { env, cwd, outputs, ...rest } = opts ?? {};
  const action: StepAction = {
    kind: "run",
    argv,
    ...(env !== undefined ? { env } : {}),
    ...(cwd !== undefined ? { cwd } : {}),
    ...(outputs !== undefined ? { outputs } : {}),
  };
  return { id, action, phase: "custom", ...rest };
}

/** The last rung, not the default (0007 §3): declared, flagged in
 *  `plan`. Declared `outputs` restore caching/satisfaction (0008 §4);
 *  without them the step always runs. */
export function shellStep(
  script: string,
  id: string,
  opts?: StepOpts & { outputs?: string[] },
): Step {
  validateResourceRefs(opts?.resources ?? [], `step '${id}'`);
  const { outputs, ...rest } = opts ?? {};
  return {
    id,
    action: {
      kind: "custom_shell",
      script,
      ...(outputs !== undefined ? { outputs } : {}),
    },
    phase: rest.phase ?? "custom",
    ...rest,
  };
}
