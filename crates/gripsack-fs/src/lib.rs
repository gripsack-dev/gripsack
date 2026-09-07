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

mod directories;
mod streamed;
pub use streamed::{atomic_copy_with_mode, publication_occurred};
pub mod fault;
pub use directories::{create_dir_all, open_or_create, remove_file, rename};
use fault::{Boundary, operation};
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
        ".{prefix}-{}-{}-{base}.tmp",
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
    operation(Boundary::DirSync, rel, || {
        rustix::fs::fsync(fd).map_err(io::Error::from)
    })
}

/// Keep an existing destination's mode across a content-only update
/// (0026 §7). A fresh destination gets the creation default; a
/// symlinked destination keeps its own semantics (the rename replaces
/// the link, never writes through it).
#[cfg(unix)]
fn preserve_mode(dir: &Dir, name: &Path, file: &cap_std::fs::File) -> io::Result<()> {
    let Ok(meta) = dir.symlink_metadata(name) else {
        return Ok(());
    };
    if !meta.is_file() {
        return Ok(());
    }
    let mode = {
        use cap_std::fs::PermissionsExt;
        meta.permissions().mode()
    };
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(cap_std::fs::Permissions::from_std(
        std::fs::Permissions::from_mode(mode),
    ))
}

#[cfg(not(unix))]
fn preserve_mode(_dir: &Dir, _name: &Path, _file: &cap_std::fs::File) -> io::Result<()> {
    Ok(())
}

/// Create a temp file with a unique sibling name, replacing a stale
/// same-named leftover from a crashed run.
fn create_temp(dir: &Dir, tmp: &Path) -> io::Result<cap_std::fs::File> {
    let mut opts = cap_std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    match dir.open_with(tmp, &opts) {
        Ok(file) => Ok(file),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            remove_file(dir, tmp)?;
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
    create_dir_all(dir, parent)?;
    let tmp = parent.join(temp_name("tmp-write", name));
    let result = (|| {
        let mut file = create_temp(dir, &tmp)?;
        operation(Boundary::Write, name, || {
            io::Write::write_all(&mut file, contents)
        })?;
        // a content update is not a mode change (0026 §7): the fresh
        // temp file would otherwise land 0644&umask, silently
        // widening a 0600 secret or dropping an exec bit on update
        operation(Boundary::Mode, name, || preserve_mode(dir, name, &file))?;
        operation(Boundary::FileSync, name, || file.sync_all())?;
        operation(Boundary::FilePublish, name, || dir.rename(&tmp, dir, name))
    })();
    if result.is_err() {
        let _ = remove_file(dir, &tmp);
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
    let _ = remove_file(dir, &tmp);
    // rustix, not Dir::symlink: cap-std validates the TARGET stays
    // inside the capability, but symlink creation writes bytes — no
    // resolution happens — and gripsack's links point at absolute
    // store paths by design (the generation flip, owned deploys).
    operation(Boundary::Symlink, link, || {
        rustix::fs::symlinkat(target, dir, &tmp).map_err(io::Error::from)
    })
    .map_err(|e| io::Error::new(e.kind(), format!("symlink {link:?} -> {target:?}: {e}")))?;
    if let Err(e) = operation(Boundary::FilePublish, link, || dir.rename(&tmp, dir, link)) {
        let _ = remove_file(dir, &tmp);
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
    // the payload's bytes AND directory structure must be durable
    // before the rename (0035 F11): a durable final name over unsynced
    // staging is a corrupt store after a power loss. The XDEV copy
    // path syncs per file already; the same-fs rename path needs the
    // recursive sync here.
    fsync_tree(home, staging)?;
    let parent = parent_rel(dest);
    create_dir_all(home, parent)?;
    match if fault::force_copy() {
        Err(io::Error::from(rustix::io::Errno::XDEV))
    } else {
        operation(Boundary::TreePublish, dest, || {
            rustix::fs::renameat(rustix::fs::CWD, staging, home, dest).map_err(io::Error::from)
        })
    } {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
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
                operation(Boundary::TreePublish, dest, || {
                    home.rename(&sibling, home, dest)
                })
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
/// followed). Permissions are preserved verbatim (0025 §G — the
/// EXDEV publish path runs read_only_files on staging BEFORE the
/// copy, so preserving modes lands the store's read-only policy and
/// exec bits exactly like the rename path does), and every file and
/// directory is fsync'd (leaves upward) so the renamed tree is fully
/// durable, not just its final name.
/// Recursively fsync a staged tree: every file's bytes, then every
/// directory leaf-up (children durable before their parents'
/// metadata). Staging lives OUTSIDE the home capability ($TMPDIR), so
/// this walks plain paths — the capability discipline governs the
/// destination side.
fn fsync_tree(_dir: &Dir, staging: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(staging)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            fsync_tree(_dir, &entry.path())?;
        } else if ty.is_file() {
            let file = std::fs::File::open(entry.path())?;
            operation(Boundary::FileSync, &entry.path(), || file.sync_all())?;
        }
        // symlinks carry no bytes; their dir entries are synced by the
        // parent fsync below
    }
    operation(Boundary::DirSync, staging, || {
        rustix::fs::fsync(std::fs::File::open(staging)?).map_err(io::Error::from)
    })
}

fn copy_into_dir(src: &Path, dst: &Dir, rel: &Path) -> io::Result<()> {
    create_dir_all(dst, rel)?;
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
            operation(Boundary::Symlink, &to, || {
                rustix::fs::symlinkat(&target, dst, &to).map_err(io::Error::from)
            })?;
        } else {
            let mut opts = cap_std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            let mut file = dst.open_with(&to, &opts)?;
            operation(Boundary::Write, &to, || {
                std::io::copy(&mut std::fs::File::open(entry.path())?, &mut file).map(|_| ())
            })?;
            operation(Boundary::Mode, &to, || {
                file.set_permissions(cap_std::fs::Permissions::from_std(
                    entry.metadata()?.permissions(),
                ))
            })?;
            operation(Boundary::FileSync, &to, || file.sync_all())?;
        }
    }
    // children durable before the dir's own metadata
    fsync_dir(dst, rel)
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
            operation(Boundary::Mode, &path, || {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode & !0o222))
            })?;
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

/// [`atomic_write`] with an exact mode on the result — recovery
/// writes, where the recorded mode must ride the rename (0027 §6):
/// temp → write → set mode → fsync → rename, so the file never exists
/// at a wider mode, not even for the rename's instant.
#[cfg(unix)]
pub fn atomic_write_with_mode(
    dir: &Dir,
    name: &Path,
    contents: &[u8],
    mode: u32,
) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let parent = parent_rel(name);
    create_dir_all(dir, parent)?;
    let tmp = parent.join(temp_name("tmp-write", name));
    let result = (|| {
        let mut file = create_temp(dir, &tmp)?;
        operation(Boundary::Write, name, || {
            io::Write::write_all(&mut file, contents)
        })?;
        operation(Boundary::Mode, name, || {
            file.set_permissions(cap_std::fs::Permissions::from_std(
                std::fs::Permissions::from_mode(mode),
            ))
        })?;
        operation(Boundary::FileSync, name, || file.sync_all())?;
        operation(Boundary::FilePublish, name, || dir.rename(&tmp, dir, name))
    })();
    if result.is_err() {
        let _ = remove_file(dir, &tmp);
    }
    result?;
    fsync_dir(dir, parent)
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
mod persistence_model;

#[cfg(test)]
mod tests;
