/** Structured source-aware diagnostics (A1-06) — the frontend half of
 *  the core's compiler-style Diagnostic (crates/gripsack-ir/src/
 *  diagnostic.rs). The eval envelope carries these to the core, which
 *  renders the same facts to the terminal and to `grip check --json`;
 *  tooling matches on `code`, never on message text.
 *
 *  Codes are allocated by the core registry only — the constants below
 *  mirror it, they do not extend it. */

import type { Span } from "./module.ts";

/** Wire shape of one label (core `Label`): the span is null when no
 *  source node carries the context and the note alone explains it. */
export interface DiagnosticLabel {
  span: Span | null;
  note: string;
}

/** Wire shape of one diagnostic — the core's eval envelope
 *  deserializes this verbatim into `gripsack_ir::Diagnostic`
 *  (`severity` is the lowercase serde spelling). */
export interface FrontendDiagnostic {
  code: string;
  severity: "error" | "warning";
  message: string;
  labels: DiagnosticLabel[];
  help?: string;
}

/** Registry codes the workspace frontend raises, mirroring
 *  `crates/gripsack-ir/src/diagnostic.rs` `codes` — new codes are
 *  allocated THERE, never here. */
export const diagnosticCodes = {
  duplicateWorkspaceOutput: "E125",
  unknownWorkspaceRef: "E126",
  workspaceCycle: "E127",
  badWorkspaceContext: "E128",
  invalidWorkspaceValue: "E130",
} as const;

/** An authoring failure carrying its structured diagnostic. Thrown
 *  instead of a plain Error where the throw site knows the precise
 *  source facts: both collision declarations, reference plus
 *  declaration spans, the mapped interpolation line. */
export class DiagnosticError extends Error {
  readonly diagnostic: FrontendDiagnostic;
  constructor(diagnostic: FrontendDiagnostic) {
    super(diagnostic.message);
    this.name = "DiagnosticError";
    this.diagnostic = diagnostic;
  }
}

/** One-label error diagnostic at a declaration span. */
export function errorAt(
  code: string,
  message: string,
  span: Span,
  note: string,
): DiagnosticError {
  return new DiagnosticError({
    code,
    severity: "error",
    message,
    labels: [{ span, note }],
  });
}

/** Source coordinates cross into Rust u32 fields. Reject malformed
 *  labels before the core deserializes or attempts to render them. */
const MAX_SOURCE_COORDINATE = 0xffff_ffff;

function sourceCoordinate(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) &&
    value >= 1 && value <= MAX_SOURCE_COORDINATE;
}

function isSpan(value: unknown): value is Span {
  if (typeof value !== "object" || value === null) return false;
  const rec = value as Record<string, unknown>;
  return typeof rec.file === "string" && rec.file.length > 0 &&
    sourceCoordinate(rec.line) &&
    (rec.col === undefined || sourceCoordinate(rec.col));
}

function isFrontendDiagnostic(value: unknown): value is FrontendDiagnostic {
  if (typeof value !== "object" || value === null) return false;
  const rec = value as Record<string, unknown>;
  return typeof rec.code === "string" &&
    (rec.severity === "error" || rec.severity === "warning") &&
    typeof rec.message === "string" &&
    Array.isArray(rec.labels) &&
    rec.labels.every((label) =>
      typeof label === "object" && label !== null &&
      typeof (label as DiagnosticLabel).note === "string" &&
      ((label as DiagnosticLabel).span === null || isSpan((label as DiagnosticLabel).span))
    ) &&
    (rec.help === undefined || typeof rec.help === "string");
}

/** Extract the diagnostic from a thrown DiagnosticError. The winning
 *  @gripsack/core pin may be a different module instance than this
 *  driver (0013 D3), so `instanceof` across that boundary is
 *  unreliable — the tagged shape is the transport contract. */
export function asDiagnostic(error: unknown): FrontendDiagnostic | undefined {
  if (!(error instanceof Error) || error.name !== "DiagnosticError") return undefined;
  const carried = (error as { diagnostic?: unknown }).diagnostic;
  return isFrontendDiagnostic(carried) ? carried : undefined;
}

/** Frames of the frontend package itself (embedded copy or a repo's
 *  pinned @gripsack/core, src or dist) are internal; the first frame
 *  outside them is the user's declaration site. Same frame rule as
 *  module.ts's callerSpan, applied to a caught error's stack. */
function isPackageFrame(path: string, selfDir: string): boolean {
  return path.startsWith(selfDir) || path.includes("/node_modules/@gripsack/core/");
}

function spanFromStack(stack: string | undefined, selfDir: string): Span | undefined {
  if (!stack) return undefined;
  for (const line of stack.split("\n").slice(1)) {
    const match = line.match(/\(?([^()\s]+):(\d+):(\d+)\)?$/);
    if (!match || !match[1]) continue;
    const file = match[1].replace(/^file:\/\//, "");
    if (!isPackageFrame(file, selfDir)) {
      return { file, line: Number(match[2]), col: Number(match[3]) };
    }
  }
  return undefined;
}

/** Wrap a plain authoring-guard Error as E130 with the user's
 *  declaration site recovered from the stack: constructor guards throw
 *  from inside the package, so the first frame outside it is the call
 *  the user wrote. Errors thrown by user code itself (first frame is
 *  theirs) and engine errors (SyntaxError/TypeError/…) are real
 *  defects — they stay tracebacks on stderr (0005 §4). */
export function authoringDiagnostic(error: unknown): FrontendDiagnostic | undefined {
  if (!(error instanceof Error) || error.name !== "Error") return undefined;
  const selfDir = new URL(".", import.meta.url).pathname;
  const frames = error.stack?.split("\n").slice(1) ?? [];
  const first = frames[0]?.match(/\(?([^()\s]+):(\d+):(\d+)\)?$/);
  if (!first || !first[1]) return undefined;
  const throwSite = first[1].replace(/^file:\/\//, "");
  if (!isPackageFrame(throwSite, selfDir)) return undefined;
  const span = spanFromStack(error.stack, selfDir);
  return {
    code: diagnosticCodes.invalidWorkspaceValue,
    severity: "error",
    message: error.message,
    labels: [{ span: span ?? null, note: "in this declaration" }],
  };
}
