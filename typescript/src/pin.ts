/** Bare import-map target: exactly the ordinary authoring API of the selected
 * package. Compiler entry points are separate, including for embedded eval. */
import { authoring as api } from "./pin-selection.ts";

export const dep = api.dep;
export const merge = api.merge;
export const symlink = api.symlink;
export const template = api.template;
export const trackedCopy = api.trackedCopy;
export const brew = api.brew;
export const fileFetch = api.fileFetch;
export const git = api.git;
export const githubRelease = api.githubRelease;
export const pixi = api.pixi;
export const pluginFetch = api.pluginFetch;
export const tarball = api.tarball;
export const hasTag = api.hasTag;
export const when = api.when;
export const defineEnv = api.defineEnv;
export const tree = api.tree;
export const module = api.module;
export const customHook = api.customHook;
export const desktopEntry = api.desktopEntry;
export const fonts = api.fonts;
export const service = api.service;
export const resource = api.resource;
export const buildStep = api.buildStep;
export const configStep = api.configStep;
export const fetchStep = api.fetchStep;
export const installStep = api.installStep;
export const runStep = api.runStep;
export const shellStep = api.shellStep;
export const step = api.step;
export const verifyBinary = api.verifyBinary;
export const verifyDeployed = api.verifyDeployed;
export const verifyFile = api.verifyFile;
export const verifyShell = api.verifyShell;
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
export const condaEnvironment = api.condaEnvironment;
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
export const pixiFromLock = api.pixiFromLock;
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
export const trackedCopyTo = api.trackedCopyTo;
export const treeFiles = api.treeFiles;
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
  Condition,
  Dependency,
  Dest,
  Edge,
  Env,
  EnvContext,
  EnvFn,
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
  Intent,
  ModuleSpec,
  ModuleValue,
  Ownership,
  PackageLayout,
  PackageSpec,
  Phase,
  PixiLockSource,
  PixiLockSpec,
  ProbeBuilder,
  ProbeKind,
  ProfileSpec,
  RecipeExecution,
  RecipeSpec,
  Resource,
  RunBashSpec,
  ScheduleSpec,
  Span,
  Step,
  StepAction,
  StepOpts,
  TaskSpec,
  TaskContext,
  WorkspaceAction,
  WorkspaceStep,
  WorkspaceStepValue,
  WorkspaceInput,
  WorkspaceInputRef,
  WorkspaceMutationLock,
  WorkspaceFileCheck,
  TreeFilesOptions,
  Trigger,
  Verify,
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
  WorkspaceSourceV6,
  WorkspaceSpec,
  WorkspaceValue,
  WorkspaceWeekday,
} from "./index.ts";
