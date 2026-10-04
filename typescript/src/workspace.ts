/** Workspace declarations (0052 A1): the supported surface for the
 *  v6 workspace frontend — pure value constructors for nine typed
 *  output variants plus the shared command/file grammar and current
 *  emitter. No global registry or import-order magic — the
 *  root `gripsack.ts` entrypoint RETURNS a {@link WorkspaceValue}
 *  built by {@link workspace}, and the driver turns that value into
 *  IR (JSON) via {@link emitWorkspaceIr}.
 *
 *  Wire shape is exactly `schema/ir/v6.json`: every node carries a
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

export { artifact, bash, bashBody, exec, hostPath, lit, packageCommand, runBash, sourcePath, outputPath } from "./workspace/commands.ts";
export type { BashBuilder, BashCommandBuilder, ExecBuilder } from "./workspace/commands.ts";
export { ensureArtifact, input, inputDirectory, inputFile, lock } from "./workspace/bindings.ts";
export { cargoPackage } from "./workspace/cargo.ts";
export type { CargoPackageSpec } from "./workspace/cargo.ts";
export {
  artifactFile,
  artifactTree,
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
export type {
  BashBody,
  CheckSpec,
  CondaEnvironmentSource,
  CondaEnvironmentSpec,
  EnvironmentSpec,
  ExecSpec,
  HookSpec,
  ImageConfig,
  ImageDestination,
  ImageOwner,
  ImageSpec,
  PackageLayout,
  PackageSpec,
  PixiLockSource,
  PixiLockSpec,
  ProfileSpec,
  RecipeExecution,
  RecipeSpec,
  FetchSource,
  RunBashSpec,
  ScheduleSpec,
  TaskSpec,
  TaskContext,
  WorkspaceAction,
  WorkspaceStep,
  WorkspaceStepValue,
  WorkspaceInput,
  WorkspaceInputRef,
  WorkspaceMutationLock,
  WorkspaceFileCheck,
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
  WorkspacePackageCommand,
  WorkspacePath,
  WorkspacePlatform,
  WorkspaceProducer,
  WorkspaceProductionPath,
  WorkspaceSourceV6,
  WorkspaceRunBashCommand,
  WorkspaceSource,
  WorkspaceSpec,
  WorkspaceValue,
  WorkspaceWeekday,
} from "./workspace/ir.ts";
export {
  check,
  condaEnvironment,
  daily,
  defineWorkspace,
  environment,
  hook,
  image,
  pkg,
  pixiFromLock,
  profile,
  provider,
  recipe,
  schedule,
  targetPlatform,
  task,
  weekly,
  workspace,
} from "./workspace/outputs.ts";

import { condaEnvironment as condaEnvironmentImpl, pixiFromLock as pixiFromLockImpl } from "./workspace/outputs.ts";

/** Conda acquisition namespace: `conda.environment({ channels, packages, platforms? })`. */
export const conda = { environment: condaEnvironmentImpl };
/** Pixi acquisition namespace: `pixi.fromLock({ manifest, lock, environment })`. */
export const pixi = { fromLock: pixiFromLockImpl };
