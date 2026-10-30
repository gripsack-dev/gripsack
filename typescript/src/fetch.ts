/** Typed fetchers (0001 §3.1, 0002 §2). */

export type Fetch =
  | { kind: "github_release"; repo: string; asset: string; version?: string; sha256?: string; base_url?: string }
  | { kind: "tarball"; url: string; sha256?: string }
  | { kind: "git"; url: string; rev?: string }
  | { kind: "file"; path: string }
  | { kind: "plugin"; name: string; args?: Record<string, unknown> };

/**
 * GitHub release fetcher. `{version.bare}` strips exactly one leading lowercase
 * v from the locked tag. Legacy `{version}` tries raw then stripped asset names.
 * The platform placeholders (0016 §D1) are:
 * `{system}` (x86_64-linux, flake-style), `{target}` (the rust
 * triple, musl for Linux), `{arch}` (x86_64), `{arch.go}`
 * (amd64), `{os}` (linux) — expanded by the core from the
 * machine's facts for the supported Linux/WSL x86_64 target.
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

