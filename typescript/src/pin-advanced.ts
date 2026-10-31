/** Explicit advanced import-map target, bound to the same pin as authoring. */
import { compiler } from "./pin-selection.ts";
export const emitWorkspaceIr = compiler.emitWorkspaceIr;
export const IR_VERSION = compiler.IR_VERSION;
export const parseInputs = compiler.parseInputs;
export const createProbeBuilder = compiler.createProbeBuilder;
export type {
  Inputs,
  ProbeRequest,
  CheckNode,
  EnvironmentNode,
  HookNode,
  PackageNode,
  ProfileNode,
  ScheduleNode,
  ServiceNode,
  TaskNode,
  WorkspaceOutputNode,
} from "./advanced.ts";
