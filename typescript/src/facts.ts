/** Host facts — captured at eval, the only place they exist (0001 §5). */

export interface HostFacts {
  os: string;
  arch: string;
  tags: string[];
}

export function currentFacts(tags: string[] = [], host?: Partial<HostFacts>): HostFacts {
  return {
    os: host?.os ?? process.platform,
    arch: host?.arch ?? process.arch,
    tags,
  };
}
