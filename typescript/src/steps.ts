/**
 * Steps — the phase building blocks inside a module (0007).
 *
 * Most modules never write steps: the declarative fields are expanded
 * into the conventional pipeline by the core. Declare steps explicitly
 * for control — ordering, resource locks, or a custom action. `steps`
 * and the declarative fields are mutually exclusive per module (E103).
 */

import type { Source } from "./sources.js";

export type Phase = "fetch" | "build" | "install" | "config" | "activate" | "custom";

export type StepAction =
  | { kind: "fetch"; source: Source }
  | { kind: "build"; spec: Record<string, unknown> }
  | { kind: "custom_shell"; script: string };

export interface Step {
  id: string;
  action: StepAction;
  needs?: string[];
  resources?: string[];
  phase?: Phase;
}

export function step(
  id: string,
  action: StepAction,
  opts?: { needs?: string[]; resources?: string[]; phase?: Phase },
): Step {
  return { id, action, ...opts };
}

/** Primitives auto-declare their contention domain in the core
 *  (pixi → `pixi-lock`, …) — `resources` is for your own shared
 *  state (0007 §4). */
export function fetchStep(
  source: Source,
  id = "fetch",
  opts?: { needs?: string[]; resources?: string[] },
): Step {
  return { id, action: { kind: "fetch", source }, phase: "fetch", ...opts };
}

export function buildStep(
  spec: Record<string, unknown>,
  id = "build",
  opts?: { needs?: string[]; resources?: string[] },
): Step {
  return { id, action: { kind: "build", spec }, phase: "build", ...opts };
}

/** The honest escape hatch: declared, flagged in `plan`, busts
 *  fine-grained caching (0007 §3). */
export function shellStep(
  script: string,
  id: string,
  opts?: { needs?: string[]; resources?: string[]; phase?: Phase },
): Step {
  return {
    id,
    action: { kind: "custom_shell", script },
    phase: opts?.phase ?? "custom",
    ...opts,
  };
}
