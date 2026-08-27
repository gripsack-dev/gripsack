/** Host facts — captured at eval, the only place they exist (0001 §5). */
import { existsSync } from "node:fs";

export interface HostFacts {
  os: string;
  arch: string;
  tags: string[];
  /** e.g. "glibc-2.36", "darwin" — binary asset selection depends on it. */
  libc: string | undefined;
}

function detectLibc(): string | undefined {
  if (process.platform === "darwin") return "darwin";
  const report = process.report?.getReport() as
    | { header?: { glibcVersionRuntime?: string } }
    | undefined;
  const glibc = report?.header?.glibcVersionRuntime;
  if (glibc) return `glibc-${glibc}`;
  // musl (alpine, the e2e gate image): the loader is the tell — bun's
  // report has no glibcVersionRuntime there. Matches the python
  // frontend's normalized "musl" (parity corpus, docker gate).
  if (existsSync(`/lib/ld-musl-${rustArch(process.arch)}.so.1`)) return "musl";
  return undefined;
}

/** Node's arch names → the IR's Rust-style names (x86_64, aarch64). */
function rustArch(arch: string): string {
  if (arch === "x64") return "x86_64";
  if (arch === "arm64") return "aarch64";
  return arch;
}

export function currentFacts(tags: string[] = [], host?: Partial<HostFacts>): HostFacts {
  return {
    os: host?.os ?? process.platform,
    arch: host?.arch ?? rustArch(process.arch),
    tags,
    libc: host?.libc ?? detectLibc(),
  };
}
