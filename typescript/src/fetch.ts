/** Typed fetchers (0001 §3.1, 0002 §2). */

export type Fetch =
  | { kind: "github_release"; repo: string; asset: string; version?: string; sha256?: string; base_url?: string }
  | { kind: "tarball"; url: string; sha256?: string }
  | { kind: "git"; url: string; rev?: string }
  | { kind: "file"; path: string }
  | { kind: "plugin"; name: string; args?: Record<string, unknown> }
  | { kind: "brew"; formula: string; version?: string; sha256?: string }
  | { kind: "pixi"; package: string; version?: string; sha256?: string };

/**
 * GitHub release fetcher. `asset` patterns accept `{version}` (the
 * tag, either v-form) and the platform placeholders (0016 §D1):
 * `{system}` (x86_64-linux, flake-style), `{target}` (the rust
 * triple, musl for linux), `{arch}` (x86_64|aarch64), `{arch.go}`
 * (amd64|arm64), `{os}` (linux|darwin) — expanded by the core from the
 * machine's facts, so one spec serves every platform.
 */
export function githubRelease(spec: {
  repo: string;
  asset: string;
  version?: string;
  sha256?: string;
  base_url?: string;
}): Fetch {
  return { kind: "github_release", ...spec };
}

/** Direct tarball/zip URL; accepts the same platform placeholders as
 *  githubRelease (0016 §D1). */
export function tarball(url: string, sha256?: string): Fetch {
  return sha256 === undefined
    ? { kind: "tarball", url }
    : { kind: "tarball", url, sha256 };
}

export function git(url: string, rev?: string): Fetch {
  // rev absent = float: the core pins the default branch's HEAD into
  // the lockfile at resolve time; `grip update` moves it (0016 §D2)
  return rev === undefined
    ? { kind: "git", url }
    : { kind: "git", url, rev };
}

export function fileFetch(path: string): Fetch {
  return { kind: "file", path };
}

/** A fetcher plugin transport (0002 §4) — `gripfetch-<name>`. */
export function pluginFetch(name: string, args?: Record<string, unknown>): Fetch {
  return args === undefined
    ? { kind: "plugin", name }
    : { kind: "plugin", name, args };
}

/** A Homebrew bottle — resolved from the formula JSON, so the pin
 *  needs no download at update time. `version` is a tripwire: a
 * mismatch fails at resolve (`grip update` to move), never a range. */
export function brew(formula: string, version?: string): Fetch {
  return version === undefined
    ? { kind: "brew", formula }
    : { kind: "brew", formula, version };
}

/** A conda package via pixi, isolated PIXI_HOME, harvested to store. */
export function pixi(pkg: string, version?: string): Fetch {
  return version === undefined
    ? { kind: "pixi", package: pkg }
    : { kind: "pixi", package: pkg, version };
}
