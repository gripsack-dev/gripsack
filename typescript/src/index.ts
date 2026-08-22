/**
 * gripsack typescript frontend — typed module DSL, emits IR.
 *
 * Modules are plain TypeScript using this package (plan/0001 §3.3,
 * 0005 §1). Evaluation collects Module objects into a graph and emits
 * the IR (JSON) the Rust core consumes. The core never executes this
 * code; it only reads the IR.
 *
 * Spans (0004 §2): `module()` captures the caller's file:line:col from
 * the V8 stack so core errors point back at the user's source.
 */

export { dep } from "./deps.js";
export type { Dependency, Edge } from "./deps.js";
export { merge, symlink, template, trackedCopy } from "./entries.js";
export type { Dest, Ownership } from "./entries.js";
export { currentFacts } from "./facts.js";
export type { HostFacts } from "./facts.js";
export { clearGraph, emitIr, IR_VERSION, register } from "./graph.js";
export { customHook, desktopEntry, fonts, service } from "./intents.js";
export type { Intent, Trigger } from "./intents.js";
export { module } from "./module.js";
export type { IrEntry, IrModule, ModuleSpec, Span } from "./module.js";
export { fileSource, git, githubRelease, pluginSource, tarball } from "./sources.js";
export type { Source } from "./sources.js";
export { buildStep, fetchStep, shellStep, step } from "./steps.js";
export type { Phase, Step, StepAction } from "./steps.js";
