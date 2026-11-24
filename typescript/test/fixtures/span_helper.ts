import { pkg, provider, tarball } from "../../src/pin.ts";

/** An authoring helper like a repo's `release()`: the constructor call is here. */
export function release(name: string) {
  return pkg(name, {
    producer: provider(tarball("file:///tmp/x.tar.gz")),
    commands: { [name]: name },
    target: { os: "linux", arch: "x86_64" },
    layout: { kind: "relocatable" },
  });
}
