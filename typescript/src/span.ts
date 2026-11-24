/** Source provenance shared by workspace declarations and host probes. */
export interface Span {
  file: string;
  line: number;
  col?: number;
}

const FRAME = /\(?([^()\s]+):(\d+):(\d+)\)?$/;

/** Frames with an absolute source path; engine frames (`ext:`, `node:`,
 *  `<anonymous>`) never name user code. */
function sourceFrames(stack: string | undefined): Span[] {
  const frames: Span[] = [];
  for (const line of (stack ?? "").split("\n").slice(1)) {
    const m = line.match(FRAME);
    if (!m || !m[1]) continue;
    const file = m[1].replace(/^file:\/\//, "");
    if (!file.startsWith("/")) continue;
    frames.push({ file, line: Number(m[2]), col: Number(m[3]) });
  }
  return frames;
}

/** Capture the declaration site of an SDK constructor call.
 *
 *  The first frame outside this package is the module that called the
 *  constructor. When that module is an authoring helper (a `release()`
 *  in lib.ts wrapping `pkg(...)`), every output would point at the same
 *  line, so prefer the first frame in a different module: the call the
 *  user wrote. A declaration written directly in the workspace file keeps
 *  its own site, since the frames beyond it are the SDK and the driver. */
export function callerSpan(): Span | undefined {
  const pkgDir = new URL(".", import.meta.url).pathname;
  const outside = sourceFrames(new Error().stack).filter(
    (frame) => !frame.file.startsWith(pkgDir) && !frame.file.includes("/node_modules/@gripsack/core/"),
  );
  const first = outside[0];
  if (!first) return undefined;
  return outside.find((frame) => frame.file !== first.file) ?? first;
}
