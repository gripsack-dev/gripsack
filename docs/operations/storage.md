# Workstation storage operations

Operator runbook for the owner's WSL/Docker workstation (local Buildbot,
Docker storage, Seagate archive). Moved verbatim from the retired
`verification/release-handoff.md` by plan 0056 F0; it is machine operations
guidance, not release evidence.

## Storage hygiene and archive relocation

Standing owner authorization `STORAGE-HYGIENE-2026-10-30` permits bounded
cleanup and verified cold relocation without another capacity-approval round.
This maintenance authorization alone does not permit discarding unique evidence,
changing admission floors, rewriting receipts, deleting data volumes or
interrupting active work.

Separate owner decision `STORAGE-FAST-96-2026-10-31` temporarily sets only the
current B: physical reserve to 96 GiB (`103079215104` bytes) while the existing
drive is in use. The system-volume 128-GiB and state-volume 64-GiB floors and
all twelve qualification gates are unchanged. This accepts less reserve for
concurrent build growth; it is not a passing qualification or a failed-gate waiver.
Restore B:'s 128-GiB reserve during the 4-TB replacement/identity update below.

1. Measure both filesystems before acting: `df`/`du` for Ubuntu and
   `Get-Volume` for Windows. The current Ubuntu VHDX lives on C:, Docker-managed
   storage on B:, and Seagate's archive on D:. Inspect the WSL registration,
   configured storage identities and `/mnt/d/CI-Archive/.ci-storage-identity.json`;
   do not infer identity from a drive letter or an existing empty directory.
2. Classify the exact paths. Cargo `target/` intermediates are rebuildable;
   executables used for a reported observation, diagnostics, source, `dist/`,
   receipts, checkpoints and backups are not disposable merely because their
   parent looks temporary. Check active compiler processes, open/mapped files
   and live container mounts. Retain active files and their parent directories.
3. Prefer clearing inactive compiler output first. Checkpoint retained
   executables, generated packages and diagnostics to a fresh Seagate directory;
   preserve modes and links in an archive, compare its contents against the
   source, then hash the completed archive. Never use broad workspace globs as
   deletion authority. Root-owned container output can be handled through a
   container bound only to the explicitly selected cache directories.
4. For duplicate CI attempt evidence or predecessor inputs, hold the existing
   controller request lock and confirm no live request/build owns the candidate.
   Validate the candidate/gate/attempt receipts, complete inventories, every
   file's size/hash and stable identities against the retained archive.
   A directory name or a successful old log is not backup evidence. Keep
   unmatched, interrupted and unbacked attempts.
5. If more room is needed, copy inactive cold artifacts to a fresh directory
   under the verified Seagate archive. Record source-to-archive paths and a
   complete inventory, preserve filesystem metadata in an archive where needed,
   read back and independently verify every retained byte before removing the
   original. Keep published release/controller admission records available at
   their required paths; do not replace live state with NTFS symlinks or assume
   an archive copy is immediately usable as restored state. Same-host external
   storage is not offline/disaster-recovery proof.
6. Save a cleanup receipt outside the deleted tree: selected paths, protected
   paths, source/retained hashes, verification results, before/after available
   bytes and the exact removals. Run the real `ci_infra storage-check` afterward.
   Never substitute lowering the 64-GiB state floor or waiving a gate.

For development compilation, use a toolchain/profile-specific Docker named
volume, not a host checkout's `target/` bind:

```sh
docker compose run --build --rm \
  -v gripsack-dev-rust198-musl:/app/target \
  -e CARGO_PROFILE_DEV_OPT_LEVEL=1 \
  -e CARGO_PROFILE_DEV_DEBUG_ASSERTIONS=true \
  -e CARGO_PROFILE_DEV_OVERFLOW_CHECKS=true \
  build cargo build --locked -p gripsack
```

Named volumes remain caches, not release evidence, and consume B: capacity.
The ordinary e2e image already uses an optimized development build with debug
assertions and overflow checks; do not mistake opt-level-zero hashing cost for
a reason to weaken runtime admission or enlarge command deadlines.

Deleting Ubuntu files does not necessarily shrink its Windows VHDX. Online
`fstrim` and an eventual offline
[DiskPart compaction](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/compact-vdisk)
are separate maintenance steps. Inspect the actual Ubuntu VHDX path first.
Compaction requires the disk detached or attached read-only; quit Docker/WSL
clients and stop the distro only in an explicit downtime window. Do not terminate
the running assistant, editor or qualification jobs to shrink the file. Report
guest free space and Windows free space separately; an unrun compaction is not
a claimed recovery.

### Docker Desktop privilege boundary

Start and stop Docker Desktop as its normal owning user. Run only the necessary
disk-maintenance command in Administrator PowerShell, detach the disk, then
leave that elevated shell before starting Docker. Do not bundle an elevated
`docker desktop start` into the compaction instructions.

If WSL reports `MountDisk/HCS/E_ACCESSDENIED`, inspect the exact VHDX named in
the error, its attributes, attachment state, owner and effective permissions.
Docker's small `main/ext4.vhdx` bootstrap and large `disk/docker_data.vhdx`
are different files. The 2026-10-30 recovery found an Administrator-owned
bootstrap whose normal account could read/write bytes but could not manage
its ACL; the data disk did not have that restriction. Saved ACLs and a narrowly
scoped, owner-approved permission repair restored normal-user startup without
resetting Docker or changing the bootstrap bytes.

Back up security descriptors before repair. Grant only the intended owning
account the required access within the actual Docker WSL directory; retain
SYSTEM and existing VM/capability entries. Protected Administrator-owned child
ACLs can require elevation even when the parent is user-owned. Never grant
Everyone full control, recursively reset the whole drive, unregister a distro,
or replace the data disk as a first response. Verify the original daemon ID,
retained volume bytes and an actual container execution afterward.

Ubuntu and Docker also have separate guest filesystems. Trimming Ubuntu's `/`
does not trim Docker's data disk. Identify the actual Docker data mount before
any discard operation; a reported guest discard does not establish Windows
space recovery. Re-measure host capacity and retain the configured admission
floor.

### Planned hardware, unchanged storage roles

The owner-selected next fast tier is a Lexar NM790 4-TB NVMe in an OWC Express
1M2 enclosure on the workstation's USB4 40-Gb/s port. It replaces the existing
fast build tier; Seagate remains the cold archive. Until it is installed and
verified, the existing drive and storage identity remain authoritative.

Use the supported Docker storage-location workflow in a quiescent window.
Retain and verify the old copy before retirement, check normal-user ACLs and
the engine/image/volume inventories, and deliberately update the recorded
physical/volume identities for the replacement device. Keeping drive letter B:
does not make a new disk match the old storage binding. Keep bounded cache
retention even with the extra capacity.

The HP mini is a future independent verification/signing boundary, not a
provisioned signer today. It should receive immutable candidates and verify
their bytes and approval evidence independently; signing keys must not enter
the shared build cache or untrusted build execution. The current unsigned
GitHub-only release authority remains unchanged until the real host and its
explicit policy are provisioned and exercised.



