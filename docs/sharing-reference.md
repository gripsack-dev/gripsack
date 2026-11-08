# Sharing reference

The complete contract of `grip package`: the signed bundle format, trust
configuration, independent dependency closures, passive mixed environments,
Conda and wheel reconstruction, OCI transport and every bound. The
[sharing guide](https://gripsack.dev/docs/sharing.html) is the short,
runnable version; this page is where the exact rules live. It moved here from
the README verbatim when the README was shortened (plan 0056 era, 0.52).

## The signed bundle

`grip package` transfers passive Linux x86_64 artifacts. It does not evaluate
TypeScript, install a task or schedule, run publisher-provided checks, or change
a personal generation. A signed package is an authorized publisher's claim,
not evidence that arbitrary code is harmless or universally relocatable.

The local bundle directory contains `manifest.json`, `envelope.json`,
`manifests/<digest>/{manifest.json,envelope.json}` for independent dependencies,
and deduplicated `blobs/<sha256>`. Package identity is SHA-256 of the exact
canonical manifest; OCI transport identity is separate. To capture a prepared native
tree, place it under `input/components/app/` and provide a specification:

```json
{
  "version": 1,
  "namespace": "example/tools",
  "publisher": "example",
  "target": {
    "os": "linux", "arch": "x86_64", "abi": {"kind": "linux"},
    "minimum_kernel": "3.2", "cpu": "x86_64"
  },
  "layout": {
    "kind": "relocatable",
    "contract": {
      "behavior": "Static executable with no producer-prefix dependencies"
    }
  },
  "components": [{
    "id": "app", "payload": null, "tree": null, "runtime": [],
    "conda": null, "external": null, "inventory": []
  }],
  "roots": ["app"],
  "commands": {"tool": {"component": "app", "executable": "bin/tool", "args": []}},
  "environment": {},
  "licenses": [],
  "redistribution_authorized": true
}
```

The packer captures bounded trees, fills exact archive/tree/path/mode inventories,
and writes canonical metadata before signing. Explicit redistribution authorization
does not infer a license: include the required notice files and list their
aggregate-relative paths in `licenses`.

```sh
grip package pack --manifest specification.json --source input \
  --output bundle --key private-seed.bin --key-id publisher-2026
grip package import --bundle bundle
grip package inspect --manifest PACKAGE_MANIFEST_SHA256
grip package export --manifest PACKAGE_MANIFEST_SHA256 --output exported-bundle
```

Signing takes an explicitly selected **raw 32-byte Ed25519 seed**, owned by the
current user with mode `0600`, not PEM or a password. Keys never enter workspace
IR or package metadata. Independent signers use standard DSSE PAE with payload
type `application/vnd.gripsack.portable.v1+json`; v1 admits one signature.
The recipient must independently configure `$GRIPSACK_HOME/sharing/trust.json`:

```json
{
  "version": 1,
  "keys": [{
    "key_id": "publisher-2026",
    "publisher": "example",
    "namespaces": ["example/tools"],
    "public_key": "BASE64_RAW_32_BYTE_ED25519_PUBLIC_KEY",
    "allowed_layouts": ["relocatable"],
    "required_validators": ["manifest-integrity", "payload-integrity", "layout-safety"]
  }],
  "revoked_keys": [],
  "revoked_manifests": []
}
```

There is no automatic trust-on-first-use. Namespace membership is exact, not a
wildcard. Key removal, key/digest revocation, or tighter layout/validator policy
blocks subsequent managed consumption; retained bytes and running processes are
not deleted or killed. The receipt reports the current local policy revision.
Offline verification cannot know about unseen remote revocations. Independently
executed extracted programs are outside subsequent Gripsack enforcement.

A local workspace can select an imported artifact with
`source.package({ manifest: PACKAGE_MANIFEST_SHA256 })` as a package provider.
Its command paths are `components/app/bin/tool` for the example above; normal
recipient-owned package/environment/profile/task declarations select exposure
and checks. Import itself grants no execution or automation authority.

Targets carry executable admission requirements, not compatibility prose:
minimum kernel, physical CPU/archspec and either the Linux kernel ABI
(static/bundled userspace) or `{"kind":"glibc","minimum":"2.28"}`. Import and
every managed consumption measure the real recipient; solve-time facts and
`CONDA_OVERRIDE_*` cannot satisfy them. A target requirement never activates a
host library path or weakens native command admission.

An independent component uses `external: {"manifest": "<digest>",
"namespace": "upstream/tools", "publisher": "upstream"}` and null local
payload/tree/Conda fields. The packer exports that already imported dependency
under the lifecycle lock, including its entire signed closure. Each independent
manifest requires its **own** namespace-scoped local publisher authorization;
the root's signature cannot impersonate an upstream package. Local component
IDs may form runtime cycles, but independent manifest references must form an
acyclic graph. Missing metadata or payloads fail offline with no registry fallback.
A relocatable aggregate embeds only compatible relocatable dependency roots.
Embedding a Conda or fixed prefix is rejected rather than relabelled relocation.
Importing a parent does not silently register its child as a standalone selection.
Native environment lowering preserves self-cycles and strongly connected runtime
groups as finite aggregate views over their members, rather than dropping their
dependencies or rejecting an admitted cycle.
Repeated environment and package references remain shared DAG nodes within one
root-authority admission. Catalog inspection, fixed-layout lookup and projection
reuse do not expand shared children into deep-owned trees; a new admission gets
its own authority context and fresh trust checks.
Transitive host-policy and portable-retention queries likewise visit each borrowed
package view once, including all-false results. Distinct policy views are not
collapsed merely because they share a content or package identity.
Within one operation, repeated receipt views may reuse a complete admission
bound to the same home, resource limits and exact local policy revision.
Pinned retained-object identity/change snapshots guard that reuse; changed,
missing or replaced inputs fail the scope. Package restoration caches belong to
one immutable expected receipt map, not merely a producer or package name.
New managed operations perform fresh admission; this is not a persistent trust
cache and does not bypass native executable or ABI checks.
Directory inode pins are shared by exact retained root across import admission
and package-view restoration; SCC bases/views do not open duplicate producer
pins. Snapshot mutation checks and expected-receipt isolation remain mandatory.

## Passive mixed environments

Use `layout: {"kind":"environment"}` in the same signed format for an ordered
package selection, including native trees, original Conda archive sets and
independently published packages. `roots` records selection order; `commands`
binds each exported name to a component, its relative executable and fixed
arguments. Native payloads retain their bytes, while Conda members are installed
at their own actual recipient prefixes. The environment's home-bound command
projection has its own realization identity—it is not a relocatable prefix tar.
Internal package receipts also bind their parent authority/materialization
context. Reusing the same signed child standalone or beneath multiple parents
therefore cannot overwrite an immutable receipt or depend on acquisition order;
the child's original portable identity and payload bytes remain unchanged.

Environment values use the native typed data subset:

```json
{
  "TOOL_MODE": {"kind": "literal", "value": "development"},
  "CMAKE_PREFIX_PATH": {"kind": "artifact", "output": "python", "selector": "."},
  "PKG_CONFIG_PATH": {"kind": "artifact", "output": "native", "selector": "share/pkgconfig"}
}
```

Artifact selectors bind to the recipient component's admitted payload through
the ordinary environment composer. They are not string templates and contain
no producer-home paths. Repository inputs, host paths, command/effect references
and reserved loader/interpreter/shell-control variables are rejected. Managed
command PATH is derived from selected typed commands. Importing the environment
sets no shell variables, launches nothing and activates no profile.
Selected child contributions compose in `roots` order (later selections win);
the enclosing signed environment overrides those values, the recipient's authored
environment overrides imported values, and an explicit task-command environment
is last. Fixed arguments compose leaf-first, then enclosing fixed arguments,
then caller argv. Duplicate exposed command aliases remain an admission error,
not an order-dependent executable substitution.
Explicit command exports may select any reachable admitted member, including a
non-root runtime dependency. That does not add the dependency to ordered
environment contributions or change their precedence.
At invocation the enclosing activation's already-composed environment is
authoritative. An exported command cannot reintroduce child-only variables from
an unselected dependency; recursive resolution carries its arguments and native
runtime scope, not an extra environment contribution set.

Select an imported environment through `source.package({manifest: digest})`
with package `layout: {kind: "prefix_materialized"}` and command paths
`bin/<exported-name>`. Ordinary recipient-owned project/profile/task declarations
then grant invocation/exposure, using the common package receipts, environment
composer, launchers and roots. The separately hashed command projection is
created only during explicit native acquisition; read-only inspection never
creates it. Imported values live in the exact command activation, not ambient
profile shell exports. No parallel environment installer is introduced.


```sh
grip package push --manifest PACKAGE_MANIFEST_SHA256 \
  --registry https://registry.example --repository team/tools --reference release
grip package pull --digest sha256:TRANSPORT_MANIFEST_SHA256 \
  --registry https://registry.example --repository team/tools
```

Push uploads every blob before the OCI artifact manifest. Pull is digest-only;
a tag is mutable discovery, not a frozen selection. The OCI envelope is **not
a runnable container image**. Registry authentication is explicit through
`--bearer-token-file` or `--basic-credentials-file` (private mode `0600` JSON
with `username` and `password`). HTTPS uses rustls. Redirects, implicit challenge
token discovery, retries and fallback are not performed. A local test registry
may use anonymous HTTP only with explicit `--insecure-loopback`.
OCI carries every independent manifest/signature pair directly, bound by digest
descriptors; successful verification never depends on optional registry referrers.
The closure budget is 128 manifests, depth 32, 16 MiB combined signed metadata,
256 components, 100,000 entries and 512 MiB total deduplicated payloads; transport
also caps its total envelope/objects and uses one 300-second operation deadline.
One remaining expansion/entry budget covers native payloads and admitted Conda/
wheel originals, relocation growth and generated scripts/metadata before helper
execution or prefix publication. Bounded private archive readers are reused for
independent result validation. Generated materialization receipts and complete
activation metadata each have a 32 MiB encoder/reader bound; encoding is bounded
incrementally and must finish before the record is published.

Supported relocation is a declared, qualified runtime contract, not binary
rewriting. General embedded-prefix replacement, arbitrary interpreter rewriting,
privileged executables and publisher-supplied installers are unsupported.
The bundled-loader fixture explicitly qualifies its argv/library/plugin/data/
child behavior; dynamic `/proc/self/exe` denotes the loader and arbitrary re-exec
is not promised. Fixed layouts require the exact admitted store destination.
The v1 path subset is printable ASCII UTF-8, with case-unambiguous inventories
and conservative component-local symlinks. Bounds are 4 MiB manifest, 6 MiB
DSSE envelope, 256 components, 100,000 entries and 512 MiB total payload; normal
acquisition expansion limits apply independently.

A Conda archive-set component carries the complete admitted lock, original
archives and the **core-approved pinned helper**, together with its exact
Gripsack/Pixi/Rattler/UV license notices. Notice descriptors are signed, verified,
retained and re-exported alongside the helper; a publisher cannot omit or replace
the core-approved notice inventory. Export never copies
an installed prefix; import reconstructs at the recipient's actual final prefix
and separately records its realization. Missing originals/helper fail offline,
without solve, repodata or network fallback. A coherent environment may also
contain explicitly hash-pinned compatible wheels, either declared alongside
Conda requirements or selected by an imported Pixi lock:

```ts
conda.environment({
  channels: ["conda-forge"],
  packages: { python: "3.12.*", "xorg-libx11": "*" },
  systemRequirements: {
    libc: { family: "glibc", version: "2.28" }, linux: "4.18",
  },
  wheels: [{
    name: "my-tool",
    version: "1.0",
    url: "https://packages.example/my_tool-1.0-py3-none-any.whl",
    sha256: approvedWheelSha256,
  }],
});
```

The Python fixture owns Tk's X11 runtime through `xorg-libx11`; it does not
borrow an undeclared host library. Missing ELF dependencies are admission
errors even when a particular command has not exercised that optional module.
Its portable manifest declares `abi: {kind: "glibc", minimum: "2.28"}` and a
matching kernel floor. A kernel-only Linux target does not authorize a dynamic
program's host GNU loader; local and external environment members keep that ABI.

Binary prefix replacement remains bounded by the original Conda placeholders.
An overlong final prefix is refused, never truncated or silently redirected;
choose a shorter `GRIPSACK_HOME` if necessary. Reconstruction does not promise
arbitrary prefix lengths.


Wheel names are canonical lowercase names; URLs are explicit HTTPS wheel
locations without credentials, query strings or fragments. Metadata, hashes,
RECORD ownership, dependencies, Python requirements and platform tags are
admitted before installation. The core-pinned helper uses the maintained UV
wheel installer offline; independent core validation binds installed bytes,
generated console entrypoints and normalized receipts to the original wheel
and actual final prefix. It never runs pip, resolves missing Python dependencies,
compiles bytecode or executes package-provided installation scripts. Binary
manylinux wheels require a compatible explicit glibc baseline, rechecked against
the consuming host. Conda/wheel ownership collisions reject rather than overwrite.

Source distributions, editable packages, executable startup files (`.pth`,
`sitecustomize` and `usercustomize`), and arbitrary wheel `.data/scripts`
remain unsupported. Standard metadata-declared console entrypoints are supported.
Pixi import rejects build/resolution policies it cannot reproduce offline:
find-links, overrides, no-binary, nondefault index strategies, filename-check
bypass and exclude-newer without upload-time evidence. A missing hash or
unsupported selected record is an error naming the record, never an omission.

Runnable qualification fixtures cover native packages (`e2e/test_package_sharing.py`),
external graphs (`e2e/test_package_external_closure.py`), original Conda archives
(`e2e/test_package_conda_offline.py`), genuinely ABI-tagged extension wheels
(`e2e/test_package_native_wheel.py`) and passive mixed environments
(`e2e/test_package_environment.py`). External registry/helper/runtime lanes use
`native_integration` and require explicit fixture preparation; selecting a lane
without its inputs fails rather than becoming a passing skip. Their existence
is not a claim that native or registry qualification has passed.
