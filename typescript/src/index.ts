/**
 * gripsack typescript frontend — typed module DSL, emits IR.
 *
 * A host is a function (0013 D5): `hosts/<name>.ts` default-exports
 * `defineEnv((ctx) => ({ tags?, modules }))`, evaluated inside a
 * sandboxed, provisioned Deno with every host observation injected
 * through the inputs envelope. The core never executes this code; it
 * only reads the IR.
 *
 * Spans (0004 §2): `module()` captures the caller's file:line:col from
 * the V8 stack so core errors point back at the user's source.
 */

export { dep } from "./deps.ts";
export type { Dependency, Edge } from "./deps.ts";
export { merge, symlink, template, trackedCopy } from "./entries.ts";
export type { Dest, Ownership } from "./entries.ts";
export type { HostFacts } from "./facts.ts";
export { brew, fileFetch, git, githubRelease, pixi, pluginFetch, tarball } from "./fetch.ts";
export type { Fetch } from "./fetch.ts";
export { hasTag, when } from "./conditions.ts";
export type { Condition, FactView } from "./conditions.ts";
export { defineEnv, emitIr, IR_VERSION, mergeTags } from "./graph.ts";
export type { Env, EnvContext, EnvFn } from "./graph.ts";
export { tree } from "./tree.ts";
export { module } from "./module.ts";
export type { IrEntry, IrModule, ModuleSpec, ModuleValue, Span } from "./module.ts";
export { customHook, desktopEntry, fonts, service } from "./intents.ts";
export type { Intent, Trigger } from "./intents.ts";
export { parseInputs } from "./inputs.ts";
export type { Inputs } from "./inputs.ts";
export { createProbeBuilder } from "./probe.ts";
export type { ProbeBuilder, ProbeKind, ProbeRequest } from "./probe.ts";
export { CORE_RESOURCES, clearResources, resource } from "./resources.ts";
export type { Resource } from "./resources.ts";
export { buildStep, configStep, fetchStep, installStep, runStep, shellStep, step } from "./steps.ts";
export type { Phase, Step, StepAction, StepOpts } from "./steps.ts";
export { verifyBinary, verifyDeployed, verifyFile, verifyShell } from "./verify.ts";
export type { Verify } from "./verify.ts";
