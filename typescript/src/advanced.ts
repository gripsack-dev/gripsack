/** Explicit compiler SDK. Ordinary authoring imports @gripsack/core instead. */
export { emitIr, IR_VERSION, mergeTags } from "./graph.ts";
export { emitWorkspaceIr } from "./workspace/emit.ts";
export { parseInputs } from "./inputs.ts";
export type { Inputs } from "./inputs.ts";
export { createProbeBuilder } from "./probe.ts";
export type { ProbeRequest } from "./probe.ts";
export type { IrEntry, IrModule } from "./module.ts";
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
