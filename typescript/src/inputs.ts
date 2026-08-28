/** The inputs envelope (0013 D4): the core's JSON document handed to
 *  the frontend as `--inputs <path>` — the ONLY channel host
 *  observations arrive through. Not argv (world-visible), not env
 *  (leaks to children). */

import type { HostFacts } from "./facts.ts";

/** The eval inputs, version 1. */
export interface Inputs {
  /** Host entrypoint name — selects `hosts/<host>.ts`. */
  host: string;
  facts: HostFacts;
  /** CLI `--tags` merged with whatever the core already knows; the
   *  entrypoint's own tags are unioned in at emit (`mergeTags`). */
  tags: string[];
  /** Bound probe results, keyed `"<kind>:<arg>"`; empty in stage 1. */
  probes: Record<string, boolean>;
  settings: Record<string, unknown>;
}

const VERSION = 1;

function asRecord(v: unknown, where: string): Record<string, unknown> {
  if (v === undefined || v === null) return {};
  if (typeof v !== "object" || Array.isArray(v)) {
    throw new Error(`inputs: '${where}' must be an object`);
  }
  return v as Record<string, unknown>;
}

function asString(v: unknown, where: string): string {
  if (typeof v !== "string") throw new Error(`inputs: '${where}' must be a string`);
  return v;
}

/**
 * Parse + validate an inputs document. `source` names the file in
 * errors. Throws on anything structurally wrong — a malformed
 * envelope is a core/frontend version skew, not a user error, so it
 * must never silently degrade.
 */
export function parseInputs(text: string, source = "inputs"): Inputs {
  let doc: unknown;
  try {
    doc = JSON.parse(text);
  } catch (e) {
    throw new Error(`${source}: not valid JSON (${(e as Error).message})`);
  }
  const root = asRecord(doc, source);
  const version = root["version"];
  if (version !== VERSION) {
    throw new Error(
      `${source}: unsupported inputs version ${String(version)} (this frontend speaks ${VERSION})`,
    );
  }
  const host = asString(root["host"], "host");
  if (host === "") throw new Error(`${source}: 'host' must not be empty`);

  const factsRec = asRecord(root["facts"], "facts");
  const libc = factsRec["libc"];
  if (libc !== undefined && libc !== null && typeof libc !== "string") {
    throw new Error(`${source}: 'facts.libc' must be a string or null`);
  }
  const facts: HostFacts = {
    os: asString(factsRec["os"], "facts.os"),
    arch: asString(factsRec["arch"], "facts.arch"),
    libc: (libc as string | null | undefined) ?? null,
    hostname: asString(factsRec["hostname"], "facts.hostname"),
  };

  const tagsRaw = root["tags"] ?? [];
  if (!Array.isArray(tagsRaw) || tagsRaw.some((t) => typeof t !== "string")) {
    throw new Error(`${source}: 'tags' must be an array of strings`);
  }

  const probesRec = asRecord(root["probes"], "probes");
  const probes: Record<string, boolean> = {};
  for (const [k, v] of Object.entries(probesRec)) {
    if (typeof v !== "boolean") {
      throw new Error(`${source}: probes['${k}'] must be a boolean`);
    }
    probes[k] = v;
  }

  return {
    host,
    facts,
    tags: [...(tagsRaw as string[])],
    probes,
    settings: asRecord(root["settings"], "settings") as Record<string, unknown>,
  };
}
