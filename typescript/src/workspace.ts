/** Workspace declarations (0052 A1): the supported surface for the
 *  v4 workspace frontend — pure value constructors for the nine typed
 *  output variants plus the shared command/file grammar, and the v4
 *  workspace emitter. No global registry, no import-order magic — the
 *  root `gripsack.ts` entrypoint RETURNS a {@link WorkspaceValue}
 *  built by {@link workspace}, and the driver turns that value into
 *  IR (JSON) via {@link emitWorkspaceIr}.
 *
 *  Wire shape is exactly `schema/ir/v4.json`: every node carries a
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
 *  (shared runtime guards), commands.ts, files.ts, outputs.ts,
 *  emit.ts. This file is the supported re-export surface. */

export { artifact, exec, hostPath, lit, packageCommand, runBash } from "./workspace/commands.ts";
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
export { emitWorkspaceIr } from "./workspace/emit.ts";
export type {
  CheckNode,
  CheckSpec,
  EnvironmentNode,
  EnvironmentSpec,
  ExecSpec,
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
  RunBashSpec,
  ScheduleNode,
  ScheduleSpec,
  TaskNode,
  TaskSpec,
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
