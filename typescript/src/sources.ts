/** Typed fetchers (0001 §3.1, 0002 §2). */

export type Source =
  | { kind: "github_release"; repo: string; asset: string; version?: string; sha256?: string; baseUrl?: string }
  | { kind: "tarball"; url: string; sha256?: string }
  | { kind: "git"; url: string; rev: string }
  | { kind: "file"; path: string }
  | { kind: "plugin"; name: string; args?: Record<string, unknown> };

export function githubRelease(spec: {
  repo: string;
  asset: string;
  version?: string;
  sha256?: string;
  baseUrl?: string;
}): Source {
  return { kind: "github_release", ...spec };
}

export function tarball(url: string, sha256?: string): Source {
  return sha256 === undefined
    ? { kind: "tarball", url }
    : { kind: "tarball", url, sha256 };
}

export function git(url: string, rev: string): Source {
  return { kind: "git", url, rev };
}

export function fileSource(path: string): Source {
  return { kind: "file", path };
}

/** A sourcerer plugin transport (0002 §4). */
export function pluginSource(name: string, args?: Record<string, unknown>): Source {
  return args === undefined
    ? { kind: "plugin", name }
    : { kind: "plugin", name, args };
}
