/** Host facts — captured at eval, the only place they exist (0001 §5). */

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
  return glibc ? `glibc-${glibc}` : undefined;
}

export function currentFacts(tags: string[] = [], host?: Partial<HostFacts>): HostFacts {
  return {
    os: host?.os ?? process.platform,
    arch: host?.arch ?? process.arch,
    tags,
    libc: host?.libc ?? detectLibc(),
  };
}
