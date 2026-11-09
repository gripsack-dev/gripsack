//! One behavioural suite run against both `RealFs` and `MemFs`: the
//! volatile semantics of every operation (S1, errors, no-follow) must agree,
//! or `MemFs` explorations would prove things about a different filesystem.
use super::*;
use std::io::ErrorKind;

fn name(text: &str) -> Name {
    Name::new(text).unwrap()
}

fn write_file<S: Storage>(fs: &S, dir: &S::Dir, at: &str, bytes: &[u8]) {
    let mut file = fs
        .create_file(dir, &name(at), Permissions::new(0o600))
        .unwrap();
    fs.write(&mut file, bytes).unwrap();
    fs.sync_file(&file).unwrap();
}

fn suite<S: Storage>(fs: &S) {
    let root = fs.root().unwrap();
    // exclusive creation (S1)
    fs.create_dir(&root, &name("d"), Permissions::new(0o755))
        .unwrap();
    let error = fs
        .create_dir(&root, &name("d"), Permissions::new(0o755))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    let d = fs.open_dir(&root, &name("d")).unwrap();
    write_file(fs, &d, "a", b"alpha");
    let error = fs
        .create_file(&d, &name("a"), Permissions::new(0o600))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);

    // writes append; reads see the whole file; modes are exact after fchmod
    let mut file = fs
        .create_file(&d, &name("b"), Permissions::new(0o600))
        .unwrap();
    fs.write(&mut file, b"be").unwrap();
    fs.write(&mut file, b"ta").unwrap();
    fs.set_mode(&file, Permissions::new(0o640)).unwrap();
    assert_eq!(fs.read_file(&file).unwrap(), b"beta");
    let meta = fs.metadata(&d, &name("b")).unwrap();
    assert_eq!(meta.kind, ObjectKind::File);
    assert_eq!(meta.len, 4);
    assert_eq!(meta.permissions, Permissions::new(0o640));

    // replacing rename keeps the source inode under the target name (S1)
    let b_object = meta.object;
    fs.rename(&d, &name("b"), &d, &name("a")).unwrap();
    assert_eq!(fs.read(&d, &name("a")).unwrap(), b"beta");
    assert_eq!(fs.metadata(&d, &name("a")).unwrap().object, b_object);
    assert_eq!(
        fs.metadata(&d, &name("b")).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(fs.read_dir(&d).unwrap(), vec![name("a")]);

    // RENAME_NOREPLACE refuses an existing target, across directories too
    write_file(fs, &root, "c", b"gamma");
    let error = fs
        .rename_noreplace(&root, &name("c"), &d, &name("a"))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    fs.rename_noreplace(&root, &name("c"), &d, &name("c"))
        .unwrap();
    assert_eq!(fs.read(&d, &name("c")).unwrap(), b"gamma");

    // symlinks are stored verbatim and never followed
    fs.symlink(&root, &name("link"), Path::new("d")).unwrap();
    assert_eq!(
        fs.read_link(&root, &name("link")).unwrap(),
        PathBuf::from("d")
    );
    assert_eq!(
        fs.metadata(&root, &name("link")).unwrap().kind,
        ObjectKind::Symlink
    );
    assert!(fs.open_dir(&root, &name("link")).is_err());
    fs.symlink(&root, &name("flink"), Path::new("d/a")).unwrap();
    assert!(fs.open_file(&root, &name("flink")).is_err());
    assert_eq!(
        fs.create_file(&root, &name("flink"), Permissions::new(0o600))
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
    assert!(fs.read_link(&d, &name("a")).is_err());

    // kind errors
    assert!(fs.open_file(&root, &name("d")).is_err());
    assert!(fs.open_dir(&d, &name("a")).is_err());
    assert!(fs.unlink(&root, &name("d")).is_err());
    assert_eq!(
        fs.remove_dir(&root, &name("d")).unwrap_err().kind(),
        ErrorKind::DirectoryNotEmpty
    );
    fs.create_dir(&root, &name("e"), Permissions::new(0o755))
        .unwrap();
    let e = fs.open_dir(&root, &name("e")).unwrap();
    write_file(fs, &e, "x", b"x");
    assert_eq!(
        fs.rename(&root, &name("d"), &root, &name("e"))
            .unwrap_err()
            .kind(),
        ErrorKind::DirectoryNotEmpty
    );
    assert!(fs.rename(&root, &name("d"), &d, &name("inside")).is_err());

    // remove_tree is unlinks and rmdirs, children first
    fs.remove_tree(&root, &name("d")).unwrap();
    fs.remove_tree(&root, &name("e")).unwrap();
    fs.unlink(&root, &name("link")).unwrap();
    fs.unlink(&root, &name("flink")).unwrap();
    assert!(fs.read_dir(&root).unwrap().is_empty());
    fs.sync_dir(&root).unwrap();
}

#[test]
fn real_fs_meets_the_suite() {
    let home = tempfile::tempdir().unwrap();
    suite(&RealFs::new(home.path()));
}

#[test]
fn mem_fs_meets_the_suite() {
    suite(&MemFs::new());
}

#[test]
fn names_are_single_components() {
    for bad in ["", ".", "..", "a/b", "/", "a\0b"] {
        assert!(Name::new(bad).is_err(), "{bad:?}");
    }
    for good in ["a", ".hidden", "...", "run.json", "a b"] {
        assert!(Name::new(good).is_ok(), "{good:?}");
    }
}
