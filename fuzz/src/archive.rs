//! Bounded archive acquisition through the REAL production decoder path.
//! Arbitrary bytes are content only: they become a sandbox-local FILE
//! fetched by a real `FetchContext` with fuzz-scale limits. No shadow
//! decoder, no input-derived path, no global env mutation.

use crate::sandbox::Sandbox;
use gripsack_fetch::{FetchContext, FetchError, FetchLimits, FetchOutcome};
use gripsack_ir::FetchSpec;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::{Component, Path};

/// 0042 E fuzz-scale acquisition limits: 64 KiB download, 1 MiB
/// expanded, 128 entries, 8 MiB decoder memory.
const DOWNLOAD: u64 = 64 * 1024;
const EXPANDED: u64 = 1024 * 1024;
const ENTRIES: usize = 128;

fn limits() -> FetchLimits {
    FetchLimits {
        concurrent: NonZeroUsize::new(1).unwrap(),
        download_bytes: NonZeroU64::new(DOWNLOAD).unwrap(),
        expanded_bytes: NonZeroU64::new(EXPANDED).unwrap(),
        archive_entries: NonZeroUsize::new(ENTRIES).unwrap(),
        decoder_bytes: NonZeroU64::new(8 * 1024 * 1024).unwrap(),
    }
}

/// Outcome shape with the harness-controlled destination suffix erased,
/// so identical inputs must produce identical results.
fn shape(result: &Result<FetchOutcome, FetchError>) -> String {
    match result {
        Ok(outcome) => format!("ok:{}", outcome.identity.as_str()),
        Err(error) => format!("err:{error}")
            .replace("out-a", "out")
            .replace("out-b", "out"),
    }
}

pub(crate) fn exercise(s: &Sandbox, input: &[u8]) {
    // The input never names a path: source and both destinations are
    // harness constants inside the sandbox. Even a core path-handling
    // bug cannot turn fuzz bytes into a host mutation target.
    const SOURCE: &str = "archive/payload.bin";
    s.write(SOURCE, input);
    let spec = FetchSpec::File {
        path: s.fixed(SOURCE).to_str().unwrap().to_owned(),
    };
    let context = FetchContext::new(limits());

    let first = context.fetch(&spec, &s.fixed("archive/out-a"), None);
    let second = context.fetch(&spec, &s.fixed("archive/out-b"), None);
    // The same bytes through the real decoder are deterministic.
    assert_eq!(shape(&first), shape(&second));

    if let Ok(outcome) = &first {
        // Payload modes are attacker-chosen data (an entry may legitimately
        // land 0o000); re-own the harness's private trees before reading
        // them, so audits assert containment, not permission luck.
        relax(&s.fixed("archive/out-a"));
        relax(&s.fixed("archive/out-b"));
        // Transport digest computed independently over the same source
        // must equal the acquired identity (same Download domain).
        let hashed = context
            .payload_hash(&spec)
            .unwrap()
            .expect("file payload hashes");
        assert_eq!(hashed.as_str(), outcome.identity.as_str());
        // Both destinations materialized identical trees.
        let (a, b) = (
            gripsack_store::canonical_tree_hash(&s.fixed("archive/out-a")).unwrap(),
            gripsack_store::canonical_tree_hash(&s.fixed("archive/out-b")).unwrap(),
        );
        assert_eq!(a.as_str(), b.as_str());
        let (entries, bytes) = audit(&s.fixed("archive/out-a"));
        assert!(
            entries <= ENTRIES,
            "{entries} extracted entries exceed the cap"
        );
        assert!(bytes <= EXPANDED, "{bytes} extracted bytes exceed the cap");
    }
    // Acquisition never rewrites its source.
    assert_eq!(std::fs::read(s.fixed(SOURCE)).unwrap(), input);
}

/// Independent containment audit of an extracted payload: entry and
/// byte caps plus link safety, re-derived here rather than trusted
/// from the decoder. Never follows symlinks — a link redirecting the
/// walk is exactly the escape being checked for.
fn audit(dest: &Path) -> (usize, u64) {
    let mut entries = 0usize;
    let mut bytes = 0u64;
    let mut pending = vec![dest.to_owned()];
    while let Some(directory) = pending.pop() {
        for child in std::fs::read_dir(&directory).unwrap() {
            let child = child.unwrap();
            entries += 1;
            let path = child.path();
            let kind = child.file_type().unwrap();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                bytes += std::fs::symlink_metadata(&path).unwrap().len();
            } else if kind.is_symlink() {
                let target = std::fs::read_link(&path).unwrap();
                assert!(
                    !target.is_absolute(),
                    "extracted link {target:?} is absolute"
                );
                assert!(
                    stays_within(dest, &path, &target),
                    "extracted link {target:?} escapes the payload"
                );
            } else {
                panic!("extracted special file {path:?}");
            }
        }
    }
    (entries, bytes)
}

/// Grant owner rwx on every regular file and directory under `root`,
/// without following symlinks. The trees are the harness's own; this only
/// undoes payload-declared permission bits so reads and walks succeed.
fn relax(root: &Path) {
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            if metadata.is_symlink() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode | 0o700))
                    .unwrap();
            }
            if metadata.is_dir() {
                pending.push(path);
            }
        }
    }
}

/// Lexically resolve `target` against the link's parent; the walk must
/// never climb above the payload root.
fn stays_within(root: &Path, link: &Path, target: &Path) -> bool {
    let parent = match link.parent().and_then(|p| p.strip_prefix(root).ok()) {
        Some(parent) => parent,
        None => return false,
    };
    let mut depth = parent.components().count() as i64;
    for part in target.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => return false,
        }
    }
    depth >= 0
}
