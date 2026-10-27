/** Bare import-map target: exactly the ordinary authoring API of the selected
 * package. Compiler entry points are separate, including for embedded eval. */
import { authoring as api } from "./pin-selection.ts";

export const fileFetch = api.fileFetch;
export const git = api.git;
export const githubRelease = api.githubRelease;
export const pixi = api.pixi;
export const pluginFetch = api.pluginFetch;
export const tarball = api.tarball;
export const hasTag = api.hasTag;
export const when = api.when;
export const artifact = api.artifact;
export const artifactFile = api.artifactFile;
export const artifactTree = api.artifactTree;
export const ensureArtifact = api.ensureArtifact;
export const input = api.input;
export const inputDirectory = api.inputDirectory;
export const inputFile = api.inputFile;
export const lock = api.lock;
export const bash = api.bash;
export const bashBody = api.bashBody;
export const cargoPackage = api.cargoPackage;
export const check = api.check;
export const conda = api.conda;
export const daily = api.daily;
export const defineWorkspace = api.defineWorkspace;
export const environment = api.environment;
export const exec = api.exec;
export const file = api.file;
export const hook = api.hook;
export const hostPath = api.hostPath;
export const identity = api.identity;
export const image = api.image;
export const lit = api.lit;
export const literalText = api.literalText;
export const managedBlock = api.managedBlock;
export const packageCommand = api.packageCommand;
export const sourcePath = api.sourcePath;
export const outputPath = api.outputPath;
export const pkg = api.pkg;
export const profile = api.profile;
export const provider = api.provider;
export const recipe = api.recipe;
export const repoFile = api.repoFile;
export const runBash = api.runBash;
export const schedule = api.schedule;
export const symlinkTo = api.symlinkTo;
export const targetPlatform = api.targetPlatform;
export const task = api.task;
export const templateText = api.templateText;
export const templateFile = api.templateFile;
export const trackedCopyTo = api.trackedCopyTo;
export const repoTree = api.repoTree;
export const weekly = api.weekly;
export const workspace = api.workspace;

export type {
  BashBody,
  BashBuilder,
  BashCommandBuilder,
  CargoPackageSpec,
  CheckSpec,
  CondaEnvironmentSource,
  CondaEnvironmentSpec,
  CondaSystemRequirements,
  HostRuntimeSpec,
  HostRuntimeRequirements,
  Condition,
  EnvironmentSpec,
  ExecBuilder,
  ExecSpec,
  FactView,
  FetchSource,
  Fetch,
  HookSpec,
  HostFacts,
  ImageConfig,
  ImageDestination,
  ImageOwner,
  ImageSpec,
  PackageLayout,
  PackageSpec,
  PixiLockSource,
  PixiLockSpec,
  ProbeBuilder,
  ProbeKind,
  ProfileSpec,
  RecipeExecution,
  RecipeSpec,
  RunBashSpec,
  ScheduleSpec,
  Span,
  TaskSpec,
  TaskContext,
  WorkspaceAction,
  WorkspaceStep,
  WorkspaceStepValue,
  WorkspaceInput,
  WorkspaceInputRef,
  WorkspaceMutationLock,
  WorkspaceFileCheck,
  ArchiveRootPolicy,
  ProfileLaunchers,
  RepoTreeSelection,
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
  WorkspaceRunBashCommand,
  WorkspaceSource,
  AcquisitionSource,
  WorkspaceSpec,
  WorkspaceValue,
  WorkspaceWeekday,
} from "./index.ts";
