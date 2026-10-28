/** Eval driver (0013 D2/D5): `deno run … src/cli.ts <repo> --inputs <path>`.
 *
 * Runs inside the core's sandbox (no env, no network, no subprocesses,
 * read-only within captured source roots + one immutable input file).
 * Imports the repository's gripsack.ts workspace with the core-injected
 * context, and prints the eval envelope on
 * stdout: {"ir": …, "diagnostics": [], "probe_requests": […]}.
 * Error diagnostics exit 1: source-aware authoring failures cross as
 * structured envelope diagnostics (A1-06, diagnostic.ts); real defects
 * stay tracebacks — the core passes stderr through untouched (0005 §4).
 */

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { asDiagnostic, authoringDiagnostic } from "./diagnostic.ts";
import { core, coreUrl } from "./pin-selection.ts";

function die(msg: string): never {
  console.error(`gripsack: ${msg}`);
  process.exit(1);
}

async function main(): Promise<void> {
  if (
    typeof core.parseInputs !== "function" ||
    typeof core.createProbeBuilder !== "function" ||
    typeof core.emitWorkspaceIr !== "function" || core.IR_VERSION !== 9
  ) {
    die(
      `the pinned @gripsack/core at ${coreUrl} must support the IR9 workspace frontend — ` +
        `update or remove the repo's node_modules/@gripsack/core pin`,
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

  // The root workspace is the sole evaluated authoring entrypoint.
  const workspaceFile = join(repo, "gripsack.ts");
  if (existsSync(workspaceFile)) {
    // Source-aware authoring failures cross as envelope diagnostics the
    // core renders like its own (terminal and --json carry the same
    // facts). Outputs are constructed at module top level, so the
    // import itself is inside the boundary; engine errors (syntax,
    // unresolved imports) rethrow to the traceback path (0005 §4).
    const { probe, requests } = core.createProbeBuilder(inputs.probes);
    try {
      const workspaceMod = (await import(pathToFileURL(workspaceFile).href)) as {
        default?: unknown;
      };
      const workspaceFn = workspaceMod.default;
      if (typeof workspaceFn !== "function") {
        die(
          "gripsack.ts must default-export defineWorkspace((ctx) => workspace({ outputs: [...] })) " +
            "(0052 A1)",
        );
      }
      const value = (workspaceFn as (ctx: unknown) => unknown)({
        facts: inputs.facts,
        tags: inputs.tags,
        probe,
        settings: inputs.settings,
      });
      if (value instanceof Promise) {
        die("gripsack.ts must synchronously return a workspace({...}) value (got a promise)");
      }
      const payload = {
        ir: JSON.parse(core.emitWorkspaceIr(value as never, inputs.facts, inputs.tags)),
        diagnostics: [],
        probe_requests: requests,
      };
      process.stdout.write(JSON.stringify(payload) + "\n");
    } catch (error) {
      const diagnostic = asDiagnostic(error) ?? authoringDiagnostic(error);
      if (diagnostic === undefined) throw error;
      process.stdout.write(
        JSON.stringify({ ir: null, diagnostics: [diagnostic], probe_requests: requests }) + "\n",
      );
      process.exit(1);
    }
    return;
  }
  die(`no gripsack.ts in ${repo} — add a workspace entrypoint`);
}

main().catch((e: unknown) => {
  // frontend tracebacks are the frontend's domain — the core passes
  // stderr through untouched (0005 §4)
  console.error(e);
  process.exit(1);
});
