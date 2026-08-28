/** Host facts — core-injected, pure data (0013 D4).
 *
 * The frontend never detects anything: one detector (the Rust core,
 * via the inputs envelope), one frontend. `os`/`arch` are the core's
 * `std::env::consts` values, `libc` is parsed there (`glibc-<ver>`,
 * `musl`, `darwin`). */

export interface HostFacts {
  os: string;
  arch: string;
  /** e.g. "glibc-2.36", "musl", "darwin" — binary asset selection
   *  depends on it; null when undetectable. */
  libc: string | null;
  hostname: string;
}
