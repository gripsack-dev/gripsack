/** v6 workspace wire types; the core retains strict historical readers. */

import type { FactView } from "../conditions.ts";
import type { HostFacts } from "../facts.ts";
import type { Fetch } from "../fetch.ts";
import type { Span } from "../module.ts";
import type { ProbeBuilder } from "../probe.ts";

// ---------------------------------------------------------------------------
// shared wire fragments (schema/ir/v5.json $defs)
// ---------------------------------------------------------------------------

/** Literal string argument or path. Valid as both. */
export interface WorkspaceLiteral {
  kind: "literal";
  value: string;
}
/** Reference to a file/tree inside another output's artifact. Valid
 *  as both an argument and a path. */
export interface WorkspaceArtifactRef {
  kind: "artifact";
  output: string;
  selector: string;
}
/** A path on the host filesystem (command cwd only). */
export interface WorkspaceHostPath {
  kind: "host";
  path: string;
}
/** A command provided by a declared package — the ONLY interpreter a
 *  literal `run_bash` body may name; ambient host shells are never
 *  discovered (A1-03). */
export interface WorkspacePackageCommand {
  kind: "package_command";
  package: string;
  command: string;
  sha256?: string;
}

export interface WorkspaceInputRef { kind: "input"; input: string }
/** Source/output paths belong to the enclosing production operation. */
export interface WorkspaceProductionPath { kind: "source" | "output"; selector: string }
export interface WorkspaceInput {
  name: string;
  span: Span;
  origin:
    | { kind: "repo_file"; path: string }
    | { kind: "repo_directory"; path: string; include: string[]; exclude?: string[] };
}
export interface WorkspaceMutationLock { scope: "user" | "store"; key: string; span: Span }
export type WorkspacePath = WorkspaceLiteral | WorkspaceArtifactRef | WorkspaceHostPath | WorkspaceProductionPath;
export type WorkspaceArg = WorkspaceLiteral | WorkspaceArtifactRef | WorkspacePackageCommand | WorkspaceInputRef | WorkspaceProductionPath;

/** Per-output requirements — never inherited from the evaluating host. */
export interface WorkspaceOsVersion {
  major: number;
  minor: number;
  patch?: number;
}
export type WorkspaceAbi = "gnu" | "musl" | "darwin";
export interface WorkspacePlatform {
  os: "linux" | "macos";
  arch: "x86_64" | "aarch64";
  abi?: WorkspaceAbi;
  minimum_os?: WorkspaceOsVersion;
}

/** A recipe may declare host access honestly or require B2's isolated
 *  Linux worker; no implicit native or ambient-host fallback. */
export type RecipeExecution =
  | { kind: "host"; access: "unconfined" }
  | { kind: "isolated_linux"; worker: "buildkit"; platform: WorkspacePlatform; toolchain: { reference: string } };

export type PackageLayout =
  | { kind: "relocatable" }
  | { kind: "fixed_prefix"; prefix: string }
  | { kind: "prefix_materialized" };

// ---------------------------------------------------------------------------
// v6 acquisition sources (wire shape: schema/ir/v6.json $defs/workspaceSourceV6)
// ---------------------------------------------------------------------------

/** Existing archive/git/github/file/plugin acquisition, wrapped with its
 *  provenance span. The legacy brew/pixi fetch kinds are rejected in v6 —
 *  superseded by conda_environment/pixi_lock sources. */
export interface FetchSource { kind: "fetch"; fetch: Fetch; span: Span }
/** A coherent binary Conda environment solved through Rattler. `platforms`
 *  omitted means resolved per requesting consumer platform. */
export interface CondaEnvironmentSource {
  kind: "conda_environment";
  channels: string[];
  packages: Record<string, string>;
  platforms?: WorkspacePlatform[];
  span: Span;
}
/** An explicit Pixi lock import: `manifest`/`lock` are workspace INPUT
 *  names, never host paths. */
export interface PixiLockSource {
  kind: "pixi_lock";
  manifest: string;
  lock: string;
  environment: string;
  span: Span;
}
/** How a v6 recipe or provider package obtains its payload. */
export type WorkspaceSourceV6 = FetchSource | CondaEnvironmentSource | PixiLockSource;

export interface WorkspaceExecCommand {
  kind: "exec";
  span: Span;
  argv: WorkspaceArg[];
  env?: Record<string, WorkspaceArg>;
  cwd?: WorkspacePath;
}

export interface WorkspaceRunBashCommand {
  kind: "run_bash";
  span: Span;
  interpreter: WorkspaceArg;
  options: ["-e", "-u", "-o", "pipefail"];
  body: string;
  env?: Record<string, WorkspaceArg>;
  cwd?: WorkspacePath;
  /** Dedent source map: line_map[generated line - 1] = source line. */
  line_map?: number[];
}

export type WorkspaceCommand = WorkspaceExecCommand | WorkspaceRunBashCommand;

export interface WorkspaceAction { kind: "ensure_artifact"; output: string; span: Span }
export type WorkspaceStep = { command: WorkspaceCommand } | { action: WorkspaceAction };
export type WorkspaceStepValue = WorkspaceCommand | WorkspaceAction | WorkspaceStep;
export interface TaskContext { kind: "host"; mutable_paths: string[] }

export type WorkspaceSource =
  | { kind: "repo_file"; path: string }
  | { kind: "artifact_file"; output: string; selector: string }
  | { kind: "tree"; output: string; include: string[]; exclude?: string[] };

export type WorkspaceContent =
  | { kind: "identity" }
  | { kind: "literal"; text: string }
  | { kind: "template"; template: string; variables: Record<string, string>; result_digest?: string };

export type WorkspaceDestination =
  | { kind: "symlink"; path: string }
  | { kind: "tracked_copy"; path: string }
  | { kind: "managed_block"; path: string; marker: string };

export interface WorkspaceFileCheck {
  check: string;
  subject: "source" | "rendered" | "deployed";
  stage: "pre_flip" | "post_link" | "post_activate";
  span: Span;
}

/** One profile file: origin, content and destination policy are
 *  orthogonal axes (A1-11). `source` may be omitted only when the
 *  content is literal text. */
export interface WorkspaceFile {
  span: Span;
  source?: WorkspaceSource;
  content: WorkspaceContent;
  destination: WorkspaceDestination;
  checks?: WorkspaceFileCheck[];
}

export type WorkspaceWeekday = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";

/** Local-time calendar trigger — daily or weekly only. Named
 *  timezones, intervals, cron and system scope are rejected, not
 *  silently admitted as inert future promises (0052 §2.2). */
export type WorkspaceCalendar =
  | { kind: "daily"; time: string }
  | { kind: "weekly"; weekday: WorkspaceWeekday; time: string };

// ---------------------------------------------------------------------------
// output nodes (wire shape: schema/ir/v5.json $defs/*Output)
// ---------------------------------------------------------------------------

export interface RecipeNode {
  name: string;
  span: Span;
  kind: "recipe";
  /** The recipe's acquisition source — the v6 tagged union
   *  (`workspaceSourceV6`); the legacy bare `{fetch,span}` shape is gone. */
  source: WorkspaceSourceV6;
  execution: RecipeExecution;
  output_kind: "file" | "tree";
  target: WorkspacePlatform;
  steps?: WorkspaceStep[];
  checks?: string[];
}

/** Where a package comes from (A1-12 resolution/acquisition
 *  separation): a recipe output reference, or a direct provider-backed
 *  acquisition carrying its own provenance — no synthetic recipe. */
export type WorkspaceProducer =
  | { kind: "recipe"; recipe: string }
  | { kind: "provider"; provider: WorkspaceSourceV6 };

export interface PackageNode {
  name: string;
  span: Span;
  kind: "package";
  producer: WorkspaceProducer;
  commands: Record<string, string>;
  runtime?: string[];
  target: WorkspacePlatform;
  layout: PackageLayout;
}

export interface EnvironmentNode {
  name: string;
  span: Span;
  kind: "environment";
  packages: string[];
  target: WorkspacePlatform;
  prefix?: string;
  env?: Record<string, WorkspaceArg>;
}

export interface TaskNode {
  name: string;
  span: Span;
  kind: "task";
  steps: WorkspaceStep[];
  context: TaskContext;
  mutation_locks?: WorkspaceMutationLock[];
  deps?: string[];
  environment?: string;
  checks?: string[];
}

export interface ScheduleNode {
  name: string;
  span: Span;
  kind: "schedule";
  task: string;
  trigger: WorkspaceCalendar;
  scope: "user";
}

export interface CheckNode {
  name: string;
  span: Span;
  kind: "check";
  run: WorkspaceCommand;
  subject: string;
}

export interface ImageOwner {
  uid: number;
  gid: number;
}

export interface ImageDestination {
  /** Absolute installation prefix inside the image, never a host destination. */
  path: string;
  owner?: ImageOwner;
}

export interface ImageConfig {
  entrypoint?: Extract<WorkspaceArg, { kind: "literal" | "package_command" }>[];
  args?: string[];
  env?: Record<string, string>;
  cwd?: string;
  user?: ImageOwner;
}

export interface ImageNode {
  name: string;
  span: Span;
  kind: "image";
  packages: string[];
  target: WorkspacePlatform;
  base?: string;
  destinations?: Record<string, ImageDestination>;
  config: Required<ImageConfig>;
}

export interface ProfileNode {
  name: string;
  span: Span;
  kind: "profile";
  files?: WorkspaceFile[];
  environment?: string;
  schedules?: string[];
  hooks?: string[];
}

export interface HookNode {
  name: string;
  span: Span;
  kind: "hook";
  run: WorkspaceCommand;
  trigger: "post_link" | "post_activate" | "on_remove";
}

export type WorkspaceOutputNode =
  | RecipeNode
  | PackageNode
  | EnvironmentNode
  | TaskNode
  | ScheduleNode
  | CheckNode
  | ImageNode
  | ProfileNode
  | HookNode;

export type WorkspaceOutputKind = WorkspaceOutputNode["kind"];

// ---------------------------------------------------------------------------
// values and specs
// ---------------------------------------------------------------------------

/** A constructed output — the value the nine constructors return and
 *  {@link workspace} collects. The brand distinguishes real output
 *  values from stray objects (a plain field, like ModuleValue's). */
export interface WorkspaceOutput<N extends WorkspaceOutputNode = WorkspaceOutputNode> {
  readonly __gripsack: "workspace_output";
  readonly ir: N;
}

/** A constructed workspace — the value `workspace()` returns and the
 *  root `gripsack.ts` entrypoint hands to the driver. */
export interface WorkspaceValue {
  readonly __gripsack: "workspace";
  readonly ir: {
    span: Span;
    name?: string;
    inputs?: WorkspaceInput[];
    mutation_locks?: WorkspaceMutationLock[];
    outputs: WorkspaceOutputNode[];
  };
}

export interface RecipeSpec {
  /** A `Fetch` (wrapped as `{"kind":"fetch",…}`), a plain
   *  {@link CondaEnvironmentSpec}/{@link PixiLockSpec}, or a pre-built
   *  `condaEnvironment(...)`/`pixiFromLock(...)` source value. */
  source: Fetch | CondaEnvironmentSpec | PixiLockSpec | WorkspaceSourceV6;
  execution: RecipeExecution;
  output_kind: "file" | "tree";
  target: WorkspacePlatform;
  steps?: readonly WorkspaceStepValue[];
  /** Publication gates — names of `check` outputs. */
  checks?: string[];
  span?: Span;
}

/** Authoring spec for a Conda environment source — see
 *  `condaEnvironment(...)`. Channels are names/URLs in priority order;
 *  packages map a lowercase name to a version MatchSpec (`"*"` allowed). */
export interface CondaEnvironmentSpec {
  channels: string[];
  packages: Record<string, string>;
  platforms?: WorkspacePlatform[];
}

/** Authoring spec for an explicit Pixi lock import — see
 *  `pixiFromLock(...)`. `manifest`/`lock` are workspace input names. */
export interface PixiLockSpec {
  manifest: string;
  lock: string;
  environment: string;
}

export interface PackageSpec {
  /** A recipe output name (sugar for the recipe ref), or a direct
   *  `provider(fetch(...))` acquisition. */
  producer: string | WorkspaceProducer;
  /** Command name → payload-relative path. */
  commands: Record<string, string>;
  /** Explicit runtime closure — names of other `package` outputs. */
  runtime?: string[];
  target: WorkspacePlatform;
  layout: PackageLayout;
  span?: Span;
}

export interface EnvironmentSpec {
  /** Member packages — names of `package` outputs, ordered. */
  packages: string[];
  target: WorkspacePlatform;
  prefix?: string;
  env?: Record<string, WorkspaceArg>;
  span?: Span;
}

export interface TaskSpec {
  steps: readonly WorkspaceStepValue[];
  context?: TaskContext;
  locks?: readonly WorkspaceMutationLock[];
  /** Unordered prerequisites — names of `task` outputs, verified
   *  successful within one invocation. */
  deps?: string[];
  /** Name of the `environment` output the task runs in. */
  environment?: string;
  /** Per-invocation postconditions — names of `check` outputs. */
  checks?: string[];
  span?: Span;
}

export interface ScheduleSpec {
  /** Name of the `task` output to register. The declaration is
   *  inert: registration lands with E3 and is rejected with an
   *  unavailable-capability diagnostic until then. */
  task: string;
  trigger: WorkspaceCalendar;
  span?: Span;
}

export interface CheckSpec {
  run: WorkspaceCommand;
  /** Name of the output this check validates. */
  subject: string;
  span?: Span;
}

export interface ImageSpec {
  packages: string[];
  target: WorkspacePlatform;
  /** Digest-pinned registry image; omitted means scratch. Base config is not inherited. */
  base?: string;
  destinations?: Record<string, ImageDestination>;
  config?: ImageConfig;
  span?: Span;
}

export interface ProfileSpec {
  files?: WorkspaceFile[];
  /** Name of the `environment` output this profile selects. */
  environment?: string;
  /** Names of `schedule` outputs registered with the profile. */
  schedules?: string[];
  /** Names of `hook` outputs. */
  hooks?: string[];
  span?: Span;
}

export interface HookSpec {
  run: WorkspaceCommand;
  trigger: "post_link" | "post_activate" | "on_remove";
  span?: Span;
}

export interface ExecSpec {
  argv: WorkspaceArg[];
  env?: Record<string, WorkspaceArg>;
  cwd?: WorkspacePath;
  span?: Span;
}

/** Authoring-only literal Bash text with the opening template's location.
 *  Emission lowers this to body + a generated-line source map; it is
 *  never serialized as an extra workspace IR node. */
export interface BashBody {
  readonly text: string;
  readonly span: Span;
}

export interface RunBashSpec {
  /** Literal text ONLY. Use bashBody`...` for multiline scripts so
   *  dedented lines point back to the actual template location.
   *  A plain string is supported for single-line bodies only. */
  body: string | BashBody;
  /** A declared package command, never an ambient host shell. The
   *  reference alone does not establish a resolved byte pin; A1-05
   *  must bind it before execution. */
  interpreter: WorkspacePackageCommand;
  env?: Record<string, WorkspaceArg>;
  cwd?: WorkspacePath;
  span?: Span;
}

export interface WorkspaceFileSpec {
  /** File origin. Optional ONLY when `content` is `literalText`. */
  source?: WorkspaceSource;
  content: WorkspaceContent;
  destination: WorkspaceDestination;
  checks?: readonly (Omit<WorkspaceFileCheck, "span"> & { span?: Span })[];
  span?: Span;
}

export interface WorkspaceSpec {
  outputs: ReadonlyArray<WorkspaceOutput | false | null | undefined>;
  name?: string;
  inputs?: readonly WorkspaceInput[];
  span?: Span;
}


/** The context a workspace entrypoint receives: every host
 *  observation arrives here — facts and tags core-injected, probes
 *  symbolic, settings reserved. No hostname selection, no global
 *  build target. */
export interface WorkspaceContext extends FactView {
  facts: HostFacts;
  tags: string[];
  probe: ProbeBuilder;
  settings: Record<string, unknown>;
}

export type WorkspaceFn = (ctx: WorkspaceContext) => WorkspaceValue;
