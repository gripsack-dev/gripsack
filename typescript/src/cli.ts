/** Eval driver (0013 D2/D5): `deno run … src/cli.ts <repo> --inputs <path>`.
 *
 * Runs inside the core's sandbox (no env, no network, no subprocesses,
 * read-only within the repo + the inputs dir + the embedded frontend).
 * Imports the repo's host entrypoint, calls its `defineEnv` function
 * with the core-injected context, and prints the eval envelope on
 * stdout: {"ir": …, "diagnostics": [], "probe_requests": […]}.
 * Error diagnostics exit 1 (tracebacks are the frontend's domain; the
 * core passes stderr through untouched, 0005 §4).
 */

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { core, coreUrl } from "./pin.ts";
import type { Env } from "./graph.ts";

function die(msg: string): never {
  console.error(`gripsack: ${msg}`);
  process.exit(1);
}

async function main(): Promise<void> {
  if (
    typeof core.parseInputs !== "function" ||
    typeof core.createProbeBuilder !== "function" ||
    typeof core.emitIr !== "function"
  ) {
    die(
      `the pinned @gripsack/core at ${coreUrl} predates the defineEnv frontend ` +
        `(0013) — update or remove the repo's node_modules/@gripsack/core pin`,
    );
  }

  const args = process.argv.slice(2);
  const repo = resolve(args[0] ?? ".");
  let inputsPath: string | undefined;
  for (let i = 1; i < args.length; i++) {
    if (args[i] === "--inputs" && args[i + 1] !== undefined) inputsPath = args[++i];
  }
  if (inputsPath === undefined) die("--inputs <path> is required (the core always passes it)");

  const inputs = core.parseInputs(readFileSync(inputsPath, "utf8"), inputsPath);
  // host entrypoint: selected by the core (hostname / [env]
  // default_host / --host), named in the inputs envelope
  const hostFile = join(repo, "hosts", `${inputs.host}.ts`);
  // existence is checked up front — deno's failure messages differ
  // across versions, so an import error must ALWAYS mean a real
  // defect in the host file (broken import, syntax error) and pass
  // through as a traceback, never masquerade as a missing host
  if (!existsSync(hostFile)) {
    // a hosts/ dir with no match must not silently yield an empty env
    // — every when(tags=[…]) module would silently drop (same rule as
    // the python frontend had, enterprise review)
    let existing: string[] = [];
    try {
      existing = readdirSync(join(repo, "hosts")).filter((f) => f.endsWith(".ts"));
    } catch {
      // no hosts dir at all
    }
    const have = existing.map((f) => f.replace(/\.ts$/, "")).join(", ");
    die(
      existing.length > 0
        ? `no hosts/${inputs.host}.ts (have: ${have}) — add the file or change the host selection`
        : `no hosts/${inputs.host}.ts and no hosts/ directory in ${repo} — add hosts/${inputs.host}.ts`,
    );
  }
  // plugin loading: the host file is runtime-selected by the repo —
  // a static import cannot exist here
  const hostMod = (await import(pathToFileURL(hostFile).href)) as { default?: unknown };

  const envFn = hostMod.default;
  if (typeof envFn !== "function") {
    die(
      `hosts/${inputs.host}.ts must default-export defineEnv((ctx) => ({ tags, modules })) ` +
        "(0013 D5)",
    );
  }

  const { probe, requests } = core.createProbeBuilder(inputs.probes);
  const env = (envFn as (ctx: unknown) => unknown)({
    facts: inputs.facts,
    tags: inputs.tags,
    probe,
    settings: inputs.settings,
  }) as Env;
  if (
    typeof env !== "object" || env === null || !Array.isArray(env.modules) ||
    (env.tags !== undefined && !Array.isArray(env.tags))
  ) {
    die(
      `hosts/${inputs.host}.ts must synchronously return { tags?, modules: [...] } ` +
        `(got ${env instanceof Promise ? "a promise — return the env directly" : typeof env})`,
    );
  }

  const tags = core.mergeTags(env.tags, inputs.tags);
  const payload = {
    ir: JSON.parse(core.emitIr(env, inputs.facts, tags)),
    diagnostics: [],
    probe_requests: requests,
  };
  process.stdout.write(JSON.stringify(payload) + "\n");
}

main().catch((e: unknown) => {
  // frontend tracebacks are the frontend's domain — the core passes
  // stderr through untouched (0005 §4)
  console.error(e);
  process.exit(1);
});
