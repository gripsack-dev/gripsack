/** v5 workspace wire types (schema/ir/v5.json); v4 is read-only in core. */

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
}

export type WorkspacePath = WorkspaceLiteral | WorkspaceArtifactRef | WorkspaceHostPath;
export type WorkspaceArg = WorkspaceLiteral | WorkspaceArtifactRef | WorkspacePackageCommand;

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
  | { kind: "isolated_linux"; worker: "buildkit" };

export type PackageLayout =
  | { kind: "relocatable" }
  | { kind: "fixed_prefix"; prefix: string };

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
  body: string;
  env?: Record<string, WorkspaceArg>;
  cwd?: WorkspacePath;
  /** Dedent source map: line_map[generated line - 1] = source line. */
  line_map?: number[];
}

export type WorkspaceCommand = WorkspaceExecCommand | WorkspaceRunBashCommand;

export type WorkspaceSource =
  | { kind: "repo_file"; path: string }
  | { kind: "artifact_file"; output: string; selector: string };

export type WorkspaceContent =
  | { kind: "identity" }
  | { kind: "literal"; text: string }
  | { kind: "template"; template: string; variables: Record<string, string> };

export type WorkspaceDestination =
  | { kind: "symlink"; path: string }
  | { kind: "tracked_copy"; path: string }
  | { kind: "managed_block"; path: string; marker: string };

/** One profile file: origin, content and destination policy are
 *  orthogonal axes (A1-11). `source` may be omitted only when the
 *  content is literal text. */
export interface WorkspaceFile {
  span: Span;
  source?: WorkspaceSource;
  content: WorkspaceContent;
  destination: WorkspaceDestination;
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
  /** The recipe's fetch carries mandatory provenance in the v5
   *  workspaceFetch wrapper; legacy modules keep the bare shape. */
  source: { fetch: Fetch; span: Span };
  execution: RecipeExecution;
  output_kind: "file" | "tree";
  target: WorkspacePlatform;
  steps?: WorkspaceCommand[];
  checks?: string[];
}

/** Where a package comes from (A1-12 resolution/acquisition
 *  separation): a recipe output reference, or a direct provider-backed
 *  acquisition carrying its own provenance — no synthetic recipe. */
export type WorkspaceProducer =
  | { kind: "recipe"; recipe: string }
  | { kind: "provider"; provider: { fetch: Fetch; span: Span } };

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
  run: WorkspaceCommand;
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

export interface ImageNode {
  name: string;
  span: Span;
  kind: "image";
  packages: string[];
  target: WorkspacePlatform;
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
    outputs: WorkspaceOutputNode[];
  };
}

export interface RecipeSpec {
  source: Fetch;
  execution: RecipeExecution;
  output_kind: "file" | "tree";
  target: WorkspacePlatform;
  steps?: WorkspaceCommand[];
  /** Publication gates — names of `check` outputs. */
  checks?: string[];
  span?: Span;
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
  run: WorkspaceCommand;
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
  /** Pinned `packageCommand("<package>", "<command>")` — never an
   *  ambient host shell. */
  interpreter: WorkspaceArg;
  env?: Record<string, WorkspaceArg>;
  cwd?: WorkspacePath;
  span?: Span;
}

export interface WorkspaceFileSpec {
  /** File origin. Optional ONLY when `content` is `literalText`. */
  source?: WorkspaceSource;
  content: WorkspaceContent;
  destination: WorkspaceDestination;
  span?: Span;
}

export interface WorkspaceSpec {
  outputs: ReadonlyArray<WorkspaceOutput | false | null | undefined>;
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
