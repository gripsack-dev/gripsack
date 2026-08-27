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
export { brew, fileFetch, git, githubRelease, pixi, pluginFetch, tarball } from "./fetch.js";
export type { Fetch } from "./fetch.js";
export { facts, hasTag, setTags, when } from "./conditions.js";
export type { Condition, Facts } from "./conditions.js";
export { clearGraph, emitIr, IR_VERSION, register } from "./graph.js";
export { tree } from "./tree.js";
export { define, defineIf, Module, module, moduleIf } from "./module.js";
export { customHook, desktopEntry, fonts, service } from "./intents.js";
export type { Intent, Trigger } from "./intents.js";
export type { IrEntry, IrModule, ModuleSpec, Span } from "./module.js";
export { CORE_RESOURCES, clearResources, resource } from "./resources.js";
export type { Resource } from "./resources.js";
export { buildStep, configStep, fetchStep, installStep, runStep, shellStep, step } from "./steps.js";
export type { Phase, Step, StepAction, StepOpts } from "./steps.js";
export { verifyBinary, verifyDeployed, verifyFile, verifyShell } from "./verify.js";
export type { Verify } from "./verify.js";
