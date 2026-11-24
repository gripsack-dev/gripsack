/** Source provenance shared by workspace declarations and host probes. */
export interface Span {
  file: string;
  line: number;
  col?: number;
}

const FRAME = /\(?([^()\s]+):(\d+):(\d+)\)?$/;
const NAMED_FRAME = /^\s*at (?:async )?(?:new )?([^\s(]+) \(/;

interface Frame extends Span {
  /** Called through a named function (`release`), not an anonymous callback. */
  named: boolean;
}

/** Frames with an absolute source path; engine frames (`ext:`, `node:`,
 *  `<anonymous>`) never name user code. */
function sourceFrames(stack: string | undefined): Frame[] {
  const frames: Frame[] = [];
  for (const line of (stack ?? "").split("\n").slice(1)) {
    const m = line.match(FRAME);
    if (!m || !m[1]) continue;
    const file = m[1].replace(/^file:\/\//, "");
    if (!file.startsWith("/")) continue;
    const name = line.match(NAMED_FRAME)?.[1];
    frames.push({
      file,
      line: Number(m[2]),
      col: Number(m[3]),
      named: name !== undefined && !name.includes("<anonymous>") && !name.startsWith("file:"),
    });
  }
  return frames;
}

/** The declaration site within a stack, given which files are the SDK's.
 *
 *  The first frame outside the SDK is the module that called the
 *  constructor. When it was reached through a named helper function (a
 *  `release()` in lib.ts wrapping `pkg(...)`), every output would point at
 *  that one line, so prefer the first frame in a different module: the
 *  call the user wrote. Declarations written inline, in an anonymous
 *  callback such as `defineWorkspace(() => ...)`, keep their own site. */
export function declarationSite(
  stack: string | undefined,
  isPackageFrame: (file: string) => boolean,
): Span | undefined {
  const outside = sourceFrames(stack).filter((frame) => !isPackageFrame(frame.file));
  const first = outside[0];
  if (!first) return undefined;
  const site = first.named ? outside.find((frame) => frame.file !== first.file) ?? first : first;
  return { file: site.file, line: site.line, ...(site.col === undefined ? {} : { col: site.col }) };
}

/** Capture the declaration site of an SDK constructor call. */
export function callerSpan(): Span | undefined {
  const pkgDir = new URL(".", import.meta.url).pathname;
  return declarationSite(
    new Error().stack,
    (file) => file.startsWith(pkgDir) || file.includes("/node_modules/@gripsack/core/"),
  );
}
