/** Pure workspace constructors for named outputs, commands and profile files.
 *
 * Root `gripsack.ts` returns a {@link WorkspaceValue}; the explicit compiler
 * entrypoint emits the strict current schema (`schema/ir/v9.json`).
 * Declarations carry provenance, reject unknown fields and stay deeply frozen.
 * Construction declares work; only the core admits and performs effects.
 *
 * Implementations live under ./workspace/ by responsibility: wire types,
 * value/target admission, commands, files, outputs and emission.
 */

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
  repoTree,
  symlinkTo,
  templateText,
  templateFile,
  trackedCopyTo,
} from "./workspace/files.ts";
export type {
  ArchiveRootPolicy,
  ProfileLaunchers,
  RepoTreeSelection,
  BashBody,
  CheckSpec,
  CondaEnvironmentSource,
  CondaEnvironmentSpec,
  CondaSystemRequirements,
  HostRuntimeSpec,
  HostRuntimeRequirements,
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
  AcquisitionSource,
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

import { condaEnvironment as condaEnvironmentImpl, pixiFromLock as pixiFromLockImpl } from "./workspace/outputs.ts";

/** Conda acquisition namespace: `conda.environment({ channels, packages, platforms? })`. */
export const conda = { environment: condaEnvironmentImpl };
/** Explicit import of an existing Pixi lockfile. */
export const pixi = { fromLock: pixiFromLockImpl };
