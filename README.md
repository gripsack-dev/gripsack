# gripsack

**Your user-space environment, in one workspace.** Declare packages, development
environments, container services, tasks and personal profiles in typed TypeScript.
Use the same named packages for project commands, managed launchers and jobs.

**Alpha · Linux x86_64 and WSL2 only.** Native Windows, macOS and ARM are outside
the supported scope.

## What you can build

- **Packages** from release archives, coherent native Conda environments or an
  explicitly imported Pixi lock. Pin inputs before using them.
- **Environments** for `grip run` and `grip shell`, without installing into a
  global prefix or deploying your personal files.
- **Services** for digest-pinned PostgreSQL, Redis and other existing Linux
  images. Explicit readiness, loopback ports and named persistent volumes.
- **Profiles** for dotfiles and environment launchers, with explicit ownership,
  drift protection, generations and rollback.
- **Tasks and checks** for deliberate finite actions and validation. Literal
  `bash` templates keep scripts readable without interpolating values as shell code.
- **Schedules** for profile-owned systemd user timers invoking captured tasks.
  Each invocation runs its checks; previous task success is never a cache hit.

Native packages, manual tasks and file-only profiles need no container runtime.
Services use Docker Engine's structured API; systemd owns calendar wakeups.
Gripsack has no resident daemon. BuildKit source production, image building and
Homebrew support are removed rather than replaced with an implicit host fallback.

## Install

Follow the [installation guide](https://gripsack.dev/docs/install.html) and
[release downloads](https://github.com/gripsack-dev/gripsack/releases). Use a
matching core and SDK archive, or the core's embedded frontend. Unsigned releases
require manual installation: the verified installer and `grip self-update`
require signatures.

## Start with one profile

Save this complete example as the required root `gripsack.ts` in a repository:

```ts
import {
  defineWorkspace, file, literalText, managedBlock, profile, workspace,
} from "@gripsack/core";

const dotfiles = profile("dotfiles", {
  files: [file({
    content: literalText("export EDITOR=vi\n"),
    destination: managedBlock("~/.profile", "gripsack-editor"),
  })],
});

export default defineWorkspace(() => workspace({ outputs: [dotfiles] }));
```

```sh
grip check
grip plan dotfiles
grip apply dotfiles
```

Review source and policy when prompted, then inspect the plan before applying.
`grip generations` lists retained generations; `grip rollback` restores previous
managed state. To deploy files an application rewrites, choose `trackedCopyTo`:
local drift is preserved by default rather than silently overwritten.

## Grow the workspace

Add named outputs to the same catalog and select what you need:

```sh
grip update                         # refresh external source pins; review the lock
grip build my-package               # acquire without personal activation
grip run --env dev -- my-command    # execute in a selected environment
grip up postgres                   # explicit service startup and readiness
grip task run my-task               # invoke; required services must be healthy
grip down postgres                 # remove the owned container, retain its volumes
```

These commands name outputs you declare; see the complete
[runnable workspaces](examples/workspaces/) and
[PostgreSQL service guide](https://gripsack.dev/docs/services.html).
Approval binds the exact captured source and policy. Git ignore rules do not
select captured files; declare cache exclusions explicitly in `env.toml` when
needed. Keep backups and recovery state until you have exercised the real tools.

`down` never deletes database volumes, and profile rollback cannot undo SQL,
backups or task effects. Changing a running service definition requires explicit
`down`/`up`; inherited credentials are resolved only at startup. Put secrets in
inherited references, not literal environment values. Docker administrators can
inspect container environment and application logs may contain sensitive data.


## Review captured changes

Semantic review is offline: it does not evaluate TypeScript, fetch packages,
start services, run tasks or change selected generations.

```sh
grip snapshot current --out /tmp/before.snapshot.json
# Make an intentional change and apply it through the ordinary workflow.
grip diff generation:1 current --json
grip snapshot current --out /tmp/after.snapshot.json
grip diff /tmp/before.snapshot.json /tmp/after.snapshot.json
grip diff current proposed:/tmp/proposed.ir.json --json
grip diff-config /tmp/before.json /tmp/after.json --format json --json
```

`proposed:` reads an already emitted, validated workspace IR file and the
repository's existing `gripsack.lock`; it does not evaluate or update either.
New profile generations retain their admitted declaration/dependency snapshot,
actual evaluation facts and probes, selected captured-input evidence, locks and
available artifact receipts. Each private immutable review artifact is retained
through the generation's existing closure and checked by `store-verify`; profile
payload identities do not include this metadata. A repeated or provenance-only
apply reuses the prior evidence when the admitted desired state is unchanged,
rather than rewriting history or redeploying files. Rollback and comparison read
the frozen evidence without evaluating today's source.

Portable snapshots retain typed evidence and dependency relationships, not raw
configuration values. Generations predating retained review artifacts remain
readable with explicit missing declarations, probes and lock evidence; they are
not filled from the current checkout. Unavailable receipts and redacted content
also remain unknown, even when comparing a snapshot with itself.
Both terminal and JSON show the same change, impact and completeness facts.
For `diff`, exit codes are **0** complete unchanged, **2** complete changed,
**3** incomplete and **1** invalid input or another error. An incomplete report
can still contain known changes; never treat it as approval or a no-op.

`grip plan dotfiles --json` is the explicit live-planning counterpart: normal
trusted evaluation supplies its declaration, then the shared operation planner
reports exactly the selected deployment operations and their bounded configuration
views. It preserves plan's success exit code; inspect `complete`, configuration
unknowns and change completeness rather than interpreting exit 0 as complete
evidence. Managed-block payloads remain separate from observed foreign bytes.

Configuration views accept strict JSON and TOML, with an explicitly labelled
bounded byte fallback for unsupported, malformed or ambiguous input. Structural
equality does not hide exact byte changes or establish application equivalence.
Raw values are omitted by default; `--local-values` explicitly enables a bounded,
redacted local display. Generic configuration is not guaranteed secret-free.
`diff-config` uses the same exit contract: 3 for incomplete structural coverage
(including byte fallback), otherwise 2 for different exact bytes and 0 for identical
bytes. Read the structured fallback, redaction and truncation fields as well.

To authorize an exact host-bound profile plan separately from portable review:

```sh
grip plan dotfiles --save /tmp/dotfiles.saved-plan.json
grip apply --saved-plan /tmp/dotfiles.saved-plan.json
```

Saving is not execution authority. The explicit apply command rechecks captured
source, locks, trust, facts, selected scope, base generation and destination
observations under the existing lifecycle admission, then uses the ordinary
planner. Changed bindings require a new plan, not a force flag or silent source
reevaluation. Private saved-plan files can contain authored literal configuration;
keep them local and never use literal service environment values for secrets.
Inherited secret references are not resolved or stored while saving. Neither
review nor saved profile application starts an unrelated persistent service.
Existing ownership protection, crash recovery and per-mutation checks still apply;
external filesystem writers can race operating-system operations.
Save outside the captured repository (as above) or in an explicitly excluded
directory: adding the plan to its own captured source would immediately stale it.
Saved application requires the existing admitted runtime and local private
admission record; it does not provision a missing evaluator or accept an imported
envelope as host-write authority.

## Share native packages without activating profiles

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

### Passive mixed environments

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

## Documentation

- [Workspaces](https://gripsack.dev/docs/workspaces.html) and
  [packages](https://gripsack.dev/docs/packages.html)
- [Environments and launchers](https://gripsack.dev/docs/environments.html)
- [BuildKit and OCI images](https://gripsack.dev/docs/builds.html)
- [Profiles and dotfiles](https://gripsack.dev/docs/adopting.html)
- [Tasks and checks](https://gripsack.dev/docs/tasks.html)
- [SDK reference](typescript/README.md) and [runnable examples](examples/workspaces/)
- [Safety and recovery](https://gripsack.dev/docs/safety.html),
  [source capture](https://gripsack.dev/docs/settings.html) and
  [architecture](https://gripsack.dev/docs/architecture.html)
- [Design records](plan/) and [changelog](CHANGELOG.md)

## Development

```sh
docker compose run --build --rm test
docker compose run --build --rm ts-test
docker compose run --build --rm e2e uv run --locked --no-sync pytest --ignore=test_persistence_matrix.py --ignore=test_golden.py
```

See [AGENTS.md](AGENTS.md) for working agreements and the
[local qualification profile](.ci/pilot.json) for release checks. Formal research
commands and their evidence are separate from practical product qualification.

MIT licensed.
