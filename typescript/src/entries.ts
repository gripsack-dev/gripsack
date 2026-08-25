/** Deployment destinations with ownership modes (0001 §3.7). */

export type Ownership = "owned" | "tracked_copy" | "merge" | "template";

export interface Dest {
  to: string;
  mode: Ownership;
  /** Template variables (mode "template" only) — `{{ name }}` in the
      payload is substituted at deploy time. */
  vars?: Record<string, string>;
  /** Comment prefix for the managed block (mode "merge" only);
      undefined infers it from the destination extension. */
  marker?: string;
}

/** Store-owned, read-only; edits go through the module. */
export const symlink = (to: string): Dest => ({ to, mode: "owned" });
/** Copied from the store; drift detected on next apply. */
export const trackedCopy = (to: string): Dest => ({ to, mode: "tracked_copy" });
/** Managed block merged into a file other tools also write. */
export const merge = (to: string, marker?: string): Dest => ({
  to,
  mode: "merge",
  ...(marker !== undefined ? { marker } : {}),
});
/** Rendered at deploy time from `{{ name }}` placeholders. */
export const template = (to: string, vars?: Record<string, string>): Dest => ({
  to,
  mode: "template",
  ...(vars ? { vars } : {}),
});
