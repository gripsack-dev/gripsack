/** Deployment destinations with ownership modes (0001 §3.7). */

export type Ownership = "owned" | "tracked_copy" | "merge" | "template";

export interface Dest {
  to: string;
  mode: Ownership;
}

/** Store-owned, read-only; edits go through the module. */
export const symlink = (to: string): Dest => ({ to, mode: "owned" });
/** Copied from the store; drift detected on next apply. */
export const trackedCopy = (to: string): Dest => ({ to, mode: "tracked_copy" });
/** Managed block merged into a file other tools also write. */
export const merge = (to: string): Dest => ({ to, mode: "merge" });
/** Rendered at activation from module variables. */
export const template = (to: string): Dest => ({ to, mode: "template" });
