//! Capability-based filesystem primitives (plan/0021).
//!
//! gripsack's write paths used to navigate by path strings:
//! `canonicalize(dest)` → validate → act on the string later. Between
//! check and use, a swapped parent symlink changes what the path
//! resolves to (TOCTOU). This crate pins the check and the use to ONE
//! directory inode: callers open a [`Dir`] capability once and every
//! operation names files RELATIVE to it — cap-std's APIs accept
//! relative names only, so code cannot escape the directory it was
//! handed.
//!
//! The primitives preserve the store's durability rules (0001 §9.2):
//!
//! - writes stage in the same directory and rename into place — a
//!   reader never sees a partial file;
//! - symlink swaps rename over a temp link — a generation flip is
//!   indivisible;
//! - file and parent dir are fsync'd before the call returns.
//!
//! The `*_at` functions take an absolute path by opening its parent as
//! a capability on the spot. They exist for incidental writes (trust
//! list, lockfile, probe receipts) that have no check-then-use window;
//! the security-relevant paths — deploy destinations, the journal,
//! generations — hold a `Dir` opened at check time and never go
//! through them.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub use cap_std; // metadata/permissions types for Dir-relative readers
pub use cap_std::fs::Dir;

/// Open `path` as a directory capability. Ambient authority lives at
/// THIS boundary only — roots are opened once and passed down; the
/// rest of the code works relative to what it was handed.
pub fn open(path: &Path) -> io::Result<Dir> {
    Dir::open_ambient_dir(path, cap_std::ambient_authority())
}

/// [`open`] for roots that may not exist yet (first-run
/// `$GRIPSACK_HOME`): create, then open.
pub fn open_or_create(path: &Path) -> io::Result<Dir> {
    std::fs::create_dir_all(path)?;
    open(path)
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Temp sibling names carry pid + a counter so concurrent writers in
/// one process never collide; a same-named leftover from a crashed
/// run is replaced below (create_new → AlreadyExists → remove, retry).
fn temp_name(prefix: &str, name: &Path) -> PathBuf {
    let base = name
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    PathBuf::from(format!(
        ".{prefix}-{}-{}-{base}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// The parent of a relative name inside a capability: `"."` for bare
/// file names, the subdirectory path otherwise.
fn parent_rel(name: &Path) -> &Path {
    name.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}
/// fsync a directory (relative to `dir`) so renames into it are
/// durable. cap-std opens directories O_PATH on Linux, and fsync on
/// an O_PATH fd is EBADF — so the target directory is reopened
/// O_RDONLY relative to the capability and THAT fd is fsync'd.
pub fn fsync_dir(dir: &Dir, rel: &Path) -> io::Result<()> {
    let fd = rustix::fs::openat(
        dir,
        rel,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(io::Error::from)?;
    rustix::fs::fsync(fd).map_err(io::Error::from)
}

/// Create a temp file with a unique sibling name, replacing a stale
/// same-named leftover from a crashed run.
fn create_temp(dir: &Dir, tmp: &Path) -> io::Result<cap_std::fs::File> {
    let mut opts = cap_std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    match dir.open_with(tmp, &opts) {
        Ok(file) => Ok(file),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            dir.remove_file(tmp)?;
            dir.open_with(tmp, &opts)
        }
        Err(e) => Err(e),
    }
}

/// Write `contents` to `name` (relative to `dir`) atomically: temp
/// file in the same directory, fsync, rename over, fsync the parent.
/// Parent directories are created as needed.
pub fn atomic_write(dir: &Dir, name: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = parent_rel(name);
    dir.create_dir_all(parent)?;
    let tmp = parent.join(temp_name("tmp-write", name));
    let result = (|| {
        let mut file = create_temp(dir, &tmp)?;
        io::Write::write_all(&mut file, contents)?;
        file.sync_all()?;
        dir.rename(&tmp, dir, name)
    })();
    if result.is_err() {
        let _ = dir.remove_file(&tmp);
    }
    result?;
    fsync_dir(dir, parent)
}

/// Atomically point `link` (relative to `dir`) at `target`, replacing
/// any existing link — the generation flip, the single indivisible
/// operation activation reduces to (0001 §9.2). Unlike
/// [`atomic_write`], parents are NOT created: callers stage the
/// layout first (preserves the pre-migration contract).
pub fn symlink_replace(dir: &Dir, link: &Path, target: &Path) -> io::Result<()> {
    let parent = parent_rel(link);
    let tmp = parent.join(temp_name("tmp-link", link));
    let _ = dir.remove_file(&tmp);
    // rustix, not Dir::symlink: cap-std validates the TARGET stays
    // inside the capability, but symlink creation writes bytes — no
    // resolution happens — and gripsack's links point at absolute
    // store paths by design (the generation flip, owned deploys).
    rustix::fs::symlinkat(target, dir, &tmp)
        .map_err(|e| io::Error::new(e.kind(), format!("symlink {link:?} -> {target:?}: {e}")))?;
    if let Err(e) = dir.rename(&tmp, dir, link) {
        let _ = dir.remove_file(&tmp);
        return Err(io::Error::new(
            e.kind(),
            format!("link {link:?} -> {target:?}: {e}"),
        ));
    }
    fsync_dir(dir, parent)
}

/// Publish a fully built directory (an absolute staging path, usually
/// under $TMPDIR) into `dest` relative to `home`. Fails if `dest`
/// exists — generations and store paths are immutable; publishing
/// twice is a bug. Payload FILES land read-only (0016 §D3): an app
/// writing through an owned symlink gets EACCES instead of silently
/// corrupting the store. Directories stay writable so repair/gc can
/// unlink (unlink needs a writable parent, not a writable file).
pub fn publish_dir(home: &Dir, staging: &Path, dest: &Path) -> io::Result<()> {
    // metadata follows symlinks, like the pre-migration `dest.exists()`
    if home.metadata(dest).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{dest:?} already exists — store paths are immutable"),
        ));
    }
    read_only_files(staging)?;
    let parent = parent_rel(dest);
    home.create_dir_all(parent)?;
    match rustix::fs::renameat(rustix::fs::CWD, staging, home, dest) {
        Ok(()) => {}
        Err(rustix::io::Errno::XDEV) => {
            // staging lives in $TMPDIR, the store under $GRIPSACK_HOME —
            // on containers /tmp is routinely a tmpfs, so EXDEV is a
            // layout fact, not a user error. The copy must still be
            // atomic: a crash mid-copy into the FINAL name would leave
            // a partial "immutable" path that every later publish
            // refuses (AlreadyExists). Copy to a temp sibling under the
            // same capability — same filesystem as dest by construction
            // — then rename. This is simpler than the pre-migration
            // string dance precisely because the sibling cannot escape
            // the store's filesystem.
            let sibling = parent.join(temp_name("publish", dest));
            let result = (|| {
                copy_into_dir(staging, home, &sibling)?;
                home.rename(&sibling, home, dest)
            })();
            if let Err(e) = result {
                let _ = home.remove_dir_all(&sibling);
                return Err(io::Error::new(
                    e.kind(),
                    format!("publish {staging:?} -> {dest:?}: {e}"),
                ));
            }
            let _ = std::fs::remove_dir_all(staging);
        }
        Err(e) => {
            return Err(io::Error::new(
                e.kind(),
                format!("publish {staging:?} -> {dest:?}: {e}"),
            ));
        }
    }
    fsync_dir(home, parent)
}

/// Recursively copy `src` (absolute) into `rel` under `dst`:
/// directories, regular files, and symlinks (recreated, never
/// followed).
fn copy_into_dir(src: &Path, dst: &Dir, rel: &Path) -> io::Result<()> {
    dst.create_dir_all(rel)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = rel.join(entry.file_name());
        if ty.is_dir() {
            copy_into_dir(&entry.path(), dst, &to)?;
        } else if ty.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            // symlinkat: payloads legitimately carry absolute-target
            // links (see symlink_replace)
            rustix::fs::symlinkat(&target, dst, &to).map_err(io::Error::from)?;
        } else {
            dst.write(&to, std::fs::read(entry.path())?)?;
        }
    }
    Ok(())
}

/// Recursively copy a directory tree by string paths: directories,
/// regular files, and symlinks (recreated, never followed). The
/// destination is created — or merged into — so a repo overlay can
/// land on a fetched payload. Used for staging trees, which live
/// outside any capability root ($TMPDIR).
pub fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            std::os::unix::fs::symlink(target, &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// chmod every regular file under `dir` to drop write bits, keeping
/// exec (0016 §D3). Symlinks untouched (their target carries perms).
#[cfg(unix)]
fn read_only_files(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            read_only_files(&path)?;
        } else {
            let mode = meta.permissions().mode();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode & !0o222))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn read_only_files(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// Atomic write to an absolute path, via a capability opened on its
/// parent on the spot. See the module docs: incidental writes only.
pub fn atomic_write_at(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let dir = open(parent)?;
    atomic_write(
        &dir,
        Path::new(path.file_name().unwrap_or_default()),
        contents,
    )
}

/// [`symlink_replace`] at an absolute path (parent opened on the spot).
pub fn symlink_replace_at(link: &Path, target: &Path) -> io::Result<()> {
    let parent = link.parent().unwrap_or_else(|| Path::new("."));
    let dir = open(parent)?;
    symlink_replace(
        &dir,
        Path::new(link.file_name().unwrap_or_default()),
        target,
    )
}

/// [`publish_dir`] with an absolute destination (parent opened on the
/// spot).
pub fn publish_dir_at(staging: &Path, dest: &Path) -> io::Result<()> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let dir = open(parent)?;
    publish_dir(
        &dir,
        staging,
        Path::new(dest.file_name().unwrap_or_default()),
    )
}

/// An exclusive flock held until the guard drops. The ONE lock
/// primitive for the whole workspace (apply lifecycle, step
/// resources, trust-file mutations, tool provisioning): the trust
/// gate prompts for as long as the user stares at it before
/// rewriting a whole file — every load-through-save needs this.
///
/// String-based by design: lock files are coordination, not a
/// check-then-use surface.
pub struct FlockGuard(std::fs::File);

impl FlockGuard {
    /// Lock `<dir>/<name>.flock` exclusively, creating as needed.
    pub fn acquire(dir: &Path, name: &str) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(dir.join(format!("{name}.flock")))?;
        flock(&file, libc::LOCK_EX)?;
        Ok(Self(file))
    }
}

impl Drop for FlockGuard {
    fn drop(&mut self) {
        let _ = flock(&self.0, libc::LOCK_UN);
    }
}

#[cfg(unix)]
fn flock(file: &std::fs::File, op: i32) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    if unsafe { libc::flock(file.as_raw_fd(), op) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn flock(_file: &std::fs::File, _op: i32) -> io::Result<()> {
    // a lock primitive that pretends is worse than none (N5)
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "flock is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
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
}
