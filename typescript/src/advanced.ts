/** Explicit compiler SDK. Ordinary authoring imports @gripsack/core instead. */
export { emitWorkspaceIr, IR_VERSION } from "./workspace/emit.ts";
export { parseInputs } from "./inputs.ts";
export type { Inputs } from "./inputs.ts";
export { createProbeBuilder } from "./probe.ts";
export type { ProbeRequest } from "./probe.ts";
export type {
  CheckNode,
  EnvironmentNode,
  HookNode,
  ImageNode,
  PackageNode,
  ProfileNode,
  RecipeNode,
  ScheduleNode,
  TaskNode,
  WorkspaceOutputNode,
} from "./workspace/ir.ts";
