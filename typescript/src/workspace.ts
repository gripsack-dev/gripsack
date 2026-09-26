/** Workspace declarations (0052 A1): the supported surface for the
 *  v5 workspace frontend — pure value constructors for nine typed
 *  output variants plus the shared command/file grammar and v5
 *  emitter. No global registry or import-order magic — the
 *  root `gripsack.ts` entrypoint RETURNS a {@link WorkspaceValue}
 *  built by {@link workspace}, and the driver turns that value into
 *  IR (JSON) via {@link emitWorkspaceIr}.
 *
 *  Wire shape is exactly `schema/ir/v5.json`: every node carries a
 *  mandatory provenance span, all structs reject unknown fields at
 *  construction time (JS callers and casts get the same boundary as
 *  the type-checker), and returned values are deeply frozen — an
 *  authoring value never changes after construction. Execution is
 *  declared, never performed here: unavailable capabilities
 *  (`isolated_linux` before B2, schedule registration before E3) are
 *  emitted explicitly and rejected by the core's admission, never
 *  silently reinterpreted by the frontend.
 *
 *  Implementation is split into cohesive modules under ./workspace/
 *  (plan/0052 §3 ~400-line review): ir.ts (wire types), validate.ts
 *  (shared runtime guards), target.ts (execution/target/layout
 *  admission), commands.ts, files.ts, outputs.ts and emit.ts.
 *  This file is the supported re-export surface. */

export { artifact, bash, bashBody, exec, hostPath, lit, packageCommand, runBash } from "./workspace/commands.ts";
export type { BashBuilder, BashCommandBuilder, ExecBuilder } from "./workspace/commands.ts";
export {
  artifactFile,
  file,
  identity,
  literalText,
  managedBlock,
  repoFile,
  symlinkTo,
  templateText,
  trackedCopyTo,
} from "./workspace/files.ts";
export { treeFiles } from "./workspace/tree.ts";
export type { TreeFilesOptions } from "./workspace/tree.ts";
export { emitWorkspaceIr } from "./workspace/emit.ts";
export type {
  BashBody,
  CheckNode,
  CheckSpec,
  EnvironmentNode,
  EnvironmentSpec,
  ExecSpec,
  HookNode,
  HookSpec,
  ImageNode,
  ImageSpec,
  PackageLayout,
  PackageNode,
  PackageSpec,
  ProfileNode,
  ProfileSpec,
  RecipeExecution,
  RecipeNode,
  RecipeSpec,
  RunBashSpec,
  ScheduleNode,
  ScheduleSpec,
  TaskNode,
  TaskSpec,
  WorkspaceAbi,
  WorkspaceArg,
  WorkspaceArtifactRef,
  WorkspaceCalendar,
  WorkspaceCommand,
  WorkspaceContent,
  WorkspaceContext,
  WorkspaceDestination,
  WorkspaceExecCommand,
  WorkspaceFile,
  WorkspaceFileSpec,
  WorkspaceFn,
  WorkspaceHostPath,
  WorkspaceLiteral,
  WorkspaceOsVersion,
  WorkspaceOutput,
  WorkspaceOutputKind,
  WorkspaceOutputNode,
  WorkspacePackageCommand,
  WorkspacePath,
  WorkspacePlatform,
  WorkspaceProducer,
  WorkspaceRunBashCommand,
  WorkspaceSource,
  WorkspaceSpec,
  WorkspaceValue,
  WorkspaceWeekday,
} from "./workspace/ir.ts";
export {
  check,
  daily,
  defineWorkspace,
  environment,
  hook,
  image,
  pkg,
  profile,
  provider,
  recipe,
  schedule,
  targetPlatform,
  task,
  weekly,
  workspace,
} from "./workspace/outputs.ts";
