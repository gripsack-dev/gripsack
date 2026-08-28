/** Minimal ambient declarations so `tsc` (the editor/typecheck story)
 * understands the deno-style tests; the deno runtime provides the
 * real, complete types when `deno test` runs. */

declare namespace Deno {
  function test(name: string, fn: () => void | Promise<void>): void;
  /** The running deno executable — used to spawn the driver under the
   * exact sandbox flags. */
  function execPath(): string;
}
