/** Source provenance shared by workspace declarations and host probes. */
export interface Span {
  file: string;
  line: number;
  col?: number;
}

/** Capture the first V8 frame outside this package. */
export function callerSpan(): Span | undefined {
  const pkgDir = new URL(".", import.meta.url).pathname;
  const stack = new Error().stack;
  if (!stack) return undefined;
  for (const line of stack.split("\n").slice(1)) {
    const m = line.match(/\(?([^()\s]+):(\d+):(\d+)\)?$/);
    if (m && m[1] && !m[1].replace(/^file:\/\//, "").startsWith(pkgDir)) {
      const file = m[1].replace(/^file:\/\//, "");
      return { file, line: Number(m[2]), col: Number(m[3]) };
    }
  }
  return undefined;
}
