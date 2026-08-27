/** Typed fetchers (0001 §3.1, 0002 §2). */

export type Fetch =
  | { kind: "github_release"; repo: string; asset: string; version?: string; sha256?: string; base_url?: string }
  | { kind: "tarball"; url: string; sha256?: string }
  | { kind: "git"; url: string; rev: string }
  | { kind: "file"; path: string }
  | { kind: "plugin"; name: string; args?: Record<string, unknown> }
  | { kind: "brew"; formula: string; sha256?: string }
  | { kind: "pixi"; package: string; version?: string; sha256?: string };

export function githubRelease(spec: {
  repo: string;
  asset: string;
  version?: string;
  sha256?: string;
  base_url?: string;
}): Fetch {
  return { kind: "github_release", ...spec };
}

export function tarball(url: string, sha256?: string): Fetch {
  return sha256 === undefined
    ? { kind: "tarball", url }
    : { kind: "tarball", url, sha256 };
}

export function git(url: string, rev: string): Fetch {
  return { kind: "git", url, rev };
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
 *  needs no download at update time. */
export function brew(formula: string): Fetch {
  return { kind: "brew", formula };
}

/** A conda package via pixi, isolated PIXI_HOME, harvested to store. */
export function pixi(pkg: string, version?: string): Fetch {
  return version === undefined
    ? { kind: "pixi", package: pkg }
    : { kind: "pixi", package: pkg, version };
}
