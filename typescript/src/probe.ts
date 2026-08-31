/** Symbolic probes (0013 D6): the sandbox cannot run effects (no
 *  `--allow-run`, no filesystem beyond the repo), so a probe call is
 *  always a REQUEST — recorded into the eval envelope for the core to
 *  bind, returning the bound value from the inputs (absent → false).
 *  Two-stage eval: stage 1 requests, the core evaluates the closed
 *  probe enum, stage 2 re-runs with `inputs.probes` populated. */

import { callerSpan } from "./module.ts";
import type { Span } from "./module.ts";

export type ProbeKind = "executable" | "file_exists";

/** A probe the core must bind before the next eval round. `name` is
 *  the executable name (PATH lookup) or the absolute file path. */
export interface ProbeRequest {
  kind: ProbeKind;
  name: string;
  span?: Span;
}

/** What `defineEnv` receives via `ctx.probe`.
 *
 * Probe a STABLE reference, never the tool's own installed presence:
 * `!probe.executable("node") && nodeModule` oscillates — installing
 * the tool makes the next eval drop the module. Probe the specific
 * system path you must not overwrite instead
 * (`probe.file_exists("/opt/vendor/bin/node")`). */
export interface ProbeBuilder {
  /** Is `<name>` an executable on PATH? */
  executable(name: string): boolean;
  /** Does the absolute `<path>` exist? */
  file_exists(path: string): boolean;
}

/** Collect requests while handing out bound answers. The driver
 *  emits `requests` in the envelope; an entry appears only when the
 *  probe is UNBOUND — a bound probe (even `false`) re-requests
 *  nothing, which is what makes two-stage eval reach a fixpoint. */
export function createProbeBuilder(
  bound: Record<string, boolean>,
): { probe: ProbeBuilder; requests: ProbeRequest[] } {
  const requests: ProbeRequest[] = [];
  const seen = new Set<string>();
  const ask = (kind: ProbeKind, name: string): boolean => {
    if (name === "") throw new Error(`probe.${kind}: name must not be empty`);
    const key = `${kind}:${name}`;
    if (!(key in bound) && !seen.has(key)) {
      seen.add(key);
      const span = callerSpan();
      requests.push(span ? { kind, name, span } : { kind, name });
    }
    return bound[key] ?? false;
  };
  return {
    probe: {
      executable: (name) => ask("executable", name),
      file_exists: (path) => ask("file_exists", path),
    },
    requests,
  };
}
