use super::*;

#[test]
fn atomic_write_lands_content_without_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let cap = open(dir.path()).unwrap();
    atomic_write(&cap, Path::new("manifest.json"), br#"{"gen": 1}"#).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("manifest.json")).unwrap(),
        br#"{"gen": 1}"#
    );
    // overwrite works, and no temp files linger
    atomic_write(&cap, Path::new("manifest.json"), br#"{"gen": 2}"#).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("manifest.json")).unwrap(),
        br#"{"gen": 2}"#
    );
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn atomic_write_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let cap = open(dir.path()).unwrap();
    atomic_write(&cap, Path::new("journal/abc.json"), b"x").unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("journal/abc.json")).unwrap(),
        b"x"
    );
}

#[test]
fn symlink_replace_flips_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let cap = open(dir.path()).unwrap();
    std::fs::create_dir_all(dir.path().join("gen/1")).unwrap();
    std::fs::create_dir_all(dir.path().join("gen/2")).unwrap();
    symlink_replace(&cap, Path::new("current"), Path::new("gen/1")).unwrap();
    assert_eq!(
        std::fs::read_link(dir.path().join("current")).unwrap(),
        PathBuf::from("gen/1")
    );
    symlink_replace(&cap, Path::new("current"), Path::new("gen/2")).unwrap();
    assert_eq!(
        std::fs::read_link(dir.path().join("current")).unwrap(),
        PathBuf::from("gen/2")
    );
    // no temp links linger
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 2);
}

#[test]
fn publish_refuses_an_existing_dest() {
    let dir = tempfile::tempdir().unwrap();
    let cap = open(dir.path()).unwrap();
    let staging = tempfile::tempdir().unwrap();
    std::fs::write(staging.path().join("payload"), b"v1").unwrap();
    publish_dir(&cap, staging.path(), Path::new("store/aaa-m")).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("store/aaa-m/payload")).unwrap(),
        b"v1"
    );
    let staging2 = tempfile::tempdir().unwrap();
    let err = publish_dir(&cap, staging2.path(), Path::new("store/aaa-m")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
}

#[test]
fn publish_lands_files_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let cap = open(dir.path()).unwrap();
    let staging = tempfile::tempdir().unwrap();
    std::fs::write(staging.path().join("payload"), b"v1").unwrap();
    publish_dir(&cap, staging.path(), Path::new("store/aaa-m")).unwrap();
    let meta = std::fs::metadata(dir.path().join("store/aaa-m/payload")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(meta.permissions().mode() & 0o222, 0);
}

#[test]
fn copy_dir_preserves_symlinks_and_merges() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("sub/f"), b"bytes").unwrap();
    std::os::unix::fs::symlink("f", src.join("sub/link")).unwrap();
    // merge: the destination already holds a fetched payload dir
    let dst = dir.path().join("dst");
    std::fs::create_dir_all(dst.join("sub")).unwrap();
    std::fs::write(dst.join("sub/payload"), b"fetched").unwrap();
    copy_dir(&src, &dst).unwrap();
    assert_eq!(std::fs::read(dst.join("sub/f")).unwrap(), b"bytes");
    assert_eq!(std::fs::read(dst.join("sub/payload")).unwrap(), b"fetched");
    let link = dst.join("sub/link");
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(link).unwrap(), Path::new("f"));
}

/// A content-only update keeps the destination's mode (0026 §7):
/// 0600 stays 0600, 0755 stays executable.
#[test]
fn atomic_write_preserves_the_destinations_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cap = open(dir.path()).unwrap();
    let secret = dir.path().join("secret");
    std::fs::write(&secret, b"v1").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    atomic_write(&cap, Path::new("secret"), b"v2").unwrap();
    let meta = std::fs::metadata(&secret).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read(&secret).unwrap(), b"v2");

    let tool = dir.path().join("tool");
    std::fs::write(&tool, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    atomic_write(&cap, Path::new("tool"), b"#!/bin/sh\n# v2\n").unwrap();
    assert_eq!(
        std::fs::metadata(&tool).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

/// Recovery-grade write: the exact mode rides the rename — the
/// file never exists at a wider mode (0027 §6).
#[test]
fn atomic_write_with_mode_lands_exact_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cap = open(dir.path()).unwrap();
    atomic_write_with_mode(&cap, Path::new("secret"), b"s", 0o600).unwrap();
    assert_eq!(
        std::fs::metadata(dir.path().join("secret"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    // and it still replaces an existing symlink (file→link→crash
    // recovery path)
    std::os::unix::fs::symlink("/elsewhere", dir.path().join("link")).unwrap();
    atomic_write_with_mode(&cap, Path::new("link"), b"s", 0o400).unwrap();
    let meta = std::fs::metadata(dir.path().join("link")).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o400);
}

/// The EXDEV fallback across a REAL filesystem boundary: staging
/// on /dev/shm (tmpfs), the store in the test tempdir. Skipped
/// where /dev/shm does not exist (macOS). Asserts the copy
/// preserves the exec bit AND the store's read-only policy
/// (applied to staging before the copy) — the 0021 copy path
/// dropped both (0025 §G).
#[test]
fn publish_cross_filesystem_preserves_modes() {
    let shm = Path::new("/dev/shm");
    if !shm.is_dir() {
        eprintln!("no /dev/shm — skipping the cross-filesystem test");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cap = open(dir.path()).unwrap();
    let staging = tempfile::tempdir_in(shm).unwrap();
    let tool = staging.path().join("tool");
    std::fs::write(&tool, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("tool", staging.path().join("tool-link")).unwrap();

    publish_dir(&cap, staging.path(), Path::new("store/exe-m")).unwrap();

    let meta = std::fs::metadata(dir.path().join("store/exe-m/tool")).unwrap();
    let mode = meta.permissions().mode();
    assert!(mode & 0o111 != 0, "exec bit must survive EXDEV: {mode:o}");
    assert_eq!(
        mode & 0o222,
        0,
        "read-only policy must survive EXDEV: {mode:o}"
    );
    assert!(
        dir.path()
            .join("store/exe-m/tool-link")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// The adversary the migration exists for (plan/0021 acceptance):
/// a thread flips the `parent` PATH between a real directory and a
/// symlink to `evil` while writes land through a capability opened
/// on the real directory. Every write must reach the pinned inode
/// — never the directory the path currently resolves to. With
/// string-path writes this test loses `evil/pwned` within
/// iterations; with the capability it can never appear.
#[test]
fn writes_stay_on_the_pinned_inode_under_parent_swap() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    let evil = root.path().join("evil");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(&evil).unwrap();
    let parent = root.path().join("parent");
    std::os::unix::fs::symlink(&real, &parent).unwrap();

    // check time: the guard validated `parent` and opened it ONCE
    let cap = open(&parent).unwrap();

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flipper = {
        let stop = stop.clone();
        let (parent, real, evil) = (parent.clone(), real.clone(), evil.clone());
        std::thread::spawn(move || {
            let mut to_evil = false;
            while !stop.load(Ordering::Relaxed) {
                let tmp = parent.with_extension("swapping");
                let target = if to_evil { &evil } else { &real };
                if std::os::unix::fs::symlink(target, &tmp).is_ok() {
                    let _ = std::fs::rename(&tmp, &parent);
                }
                to_evil = !to_evil;
            }
        })
    };

    for i in 0..200 {
        atomic_write(&cap, Path::new("data"), format!("write {i}").as_bytes()).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    flipper.join().unwrap();

    assert!(
        !evil.join("data").exists(),
        "a write escaped the pinned directory inode into the swapped-in target"
    );
    assert_eq!(std::fs::read(real.join("data")).unwrap(), b"write 199");
}
