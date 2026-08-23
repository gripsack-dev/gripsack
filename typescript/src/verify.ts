/** Verify checks — smoke contracts, not a test framework (0007 §verify). */

export type Verify =
  | { kind: "binary_runs"; path: string; args?: string[] }
  | { kind: "file_exists"; path: string }
  | { kind: "shell"; script: string }
  | { kind: "file_deployed"; path: string };

/** A built binary runs (default sanity: `--version`-style). */
export function verifyBinary(path: string, args?: string[]): Verify {
  return args === undefined
    ? { kind: "binary_runs", path }
    : { kind: "binary_runs", path, args };
}

export function verifyFile(path: string): Verify {
  return { kind: "file_exists", path };
}

export function verifyShell(script: string): Verify {
  return { kind: "shell", script };
}

/** Check a deployed *destination* — for config-only modules, where
 *  payload-relative verifies don't apply. */
export function verifyDeployed(path: string): Verify {
  return { kind: "file_deployed", path };
}
