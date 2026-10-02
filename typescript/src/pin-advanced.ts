/** Explicit advanced import-map target, bound to the same pin as authoring. */
import { compiler } from "./pin-selection.ts";
export const emitIr = compiler.emitIr;
export const emitWorkspaceIr = compiler.emitWorkspaceIr;
export const IR_VERSION = compiler.IR_VERSION;
export const mergeTags = compiler.mergeTags;
export const parseInputs = compiler.parseInputs;
export const createProbeBuilder = compiler.createProbeBuilder;
export type {
  Inputs,
  ProbeRequest,
  IrEntry,
  IrModule,
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
} from "./advanced.ts";
