//! [`Storage`] over the `*at` syscalls (rustix), one directory descriptor
//! per pinned directory. Every effect passes the shared fault boundary
//! (`gripsack_fs::fault`), so the debug-build trace/cut controls
//! (`GRIPSACK_FS_TRACE`, `GRIPSACK_FS_CUT`) see these calls exactly like
//! the legacy primitives.
use super::{Metadata, Name, ObjectId, ObjectKind, Permissions, Storage};
use gripsack_fs::fault::{Boundary, operation};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// The real filesystem below one ambient root.
#[derive(Debug, Clone)]
pub struct RealFs {
    root: PathBuf,
}

/// A read-only directory descriptor: a valid `*at` base and fsync-able,
/// unlike Linux's `O_PATH` handles.
#[derive(Debug)]
pub struct RealDir(OwnedFd);

#[derive(Debug)]
pub struct RealFile(OwnedFd);

impl RealFs {
    /// Ambient authority lives here only: the root is resolved once, every
    /// later operation is relative to a descriptor.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);

#[allow(clippy::unnecessary_cast)] // RawMode is u16 on Apple targets
fn mode(permissions: Permissions) -> Mode {
    Mode::from_raw_mode(permissions.bits() as rustix::fs::RawMode)
}

fn label(name: &Name) -> &Path {
    name.as_path()
}

impl Storage for RealFs {
    type Dir = RealDir;
    type File = RealFile;

    fn root(&self) -> io::Result<RealDir> {
        Ok(RealDir(rustix::fs::open(
            &self.root,
            DIR_FLAGS,
            Mode::empty(),
        )?))
    }

    fn open_dir(&self, parent: &RealDir, name: &Name) -> io::Result<RealDir> {
        operation(Boundary::Read, label(name), || {
            Ok(RealDir(rustix::fs::openat(
                &parent.0,
                name.as_os_str(),
                DIR_FLAGS | OFlags::NOFOLLOW,
                Mode::empty(),
            )?))
        })
    }

    fn create_dir(
        &self,
        parent: &RealDir,
        name: &Name,
        permissions: Permissions,
    ) -> io::Result<()> {
        operation(Boundary::Mkdir, label(name), || {
            Ok(rustix::fs::mkdirat(
                &parent.0,
                name.as_os_str(),
                mode(permissions),
            )?)
        })
    }

    fn remove_dir(&self, parent: &RealDir, name: &Name) -> io::Result<()> {
        operation(Boundary::Unlink, label(name), || {
            Ok(rustix::fs::unlinkat(
                &parent.0,
                name.as_os_str(),
                AtFlags::REMOVEDIR,
            )?)
        })
    }

    fn create_file(
        &self,
        parent: &RealDir,
        name: &Name,
        permissions: Permissions,
    ) -> io::Result<RealFile> {
        // Creation is where the new name begins; the fault vocabulary has no
        // separate create boundary, so it is attributed to the write.
        operation(Boundary::Write, label(name), || {
            Ok(RealFile(rustix::fs::openat(
                &parent.0,
                name.as_os_str(),
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                mode(permissions),
            )?))
        })
    }

    fn open_file(&self, parent: &RealDir, name: &Name) -> io::Result<RealFile> {
        operation(Boundary::Read, label(name), || {
            // O_NONBLOCK: a FIFO at `name` is refused below instead of
            // blocking the open until a writer appears (as `gripsack-fs`
            // does for special-file admission). It is inert on regular files.
            let fd = rustix::fs::openat(
                &parent.0,
                name.as_os_str(),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            match FileType::from_raw_mode(rustix::fs::fstat(&fd)?.st_mode) {
                FileType::RegularFile => Ok(RealFile(fd)),
                FileType::Directory => Err(Errno::ISDIR.into()),
                _ => Err(Errno::INVAL.into()),
            }
        })
    }

    fn write(&self, file: &mut RealFile, mut bytes: &[u8]) -> io::Result<()> {
        operation(Boundary::Write, Path::new("<file>"), || {
            while !bytes.is_empty() {
                match rustix::io::write(&file.0, bytes) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => bytes = &bytes[n..],
                    Err(Errno::INTR) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        })
    }

    fn set_mode(&self, file: &RealFile, permissions: Permissions) -> io::Result<()> {
        operation(Boundary::Mode, Path::new("<file>"), || {
            Ok(rustix::fs::fchmod(&file.0, mode(permissions))?)
        })
    }

    fn read_file(&self, file: &RealFile) -> io::Result<Vec<u8>> {
        operation(Boundary::Read, Path::new("<file>"), || {
            let mut out = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                match rustix::io::pread(&file.0, &mut chunk, out.len() as u64) {
                    Ok(0) => return Ok(out),
                    Ok(n) => out.extend_from_slice(&chunk[..n]),
                    Err(Errno::INTR) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        })
    }

    fn sync_file(&self, file: &RealFile) -> io::Result<()> {
        operation(Boundary::FileSync, Path::new("<file>"), || {
            Ok(rustix::fs::fsync(&file.0)?)
        })
    }

    fn sync_dir(&self, dir: &RealDir) -> io::Result<()> {
        operation(Boundary::DirSync, Path::new("<dir>"), || {
            Ok(rustix::fs::fsync(&dir.0)?)
        })
    }

    fn rename(
        &self,
        from_dir: &RealDir,
        from: &Name,
        to_dir: &RealDir,
        to: &Name,
    ) -> io::Result<()> {
        operation(Boundary::FilePublish, label(to), || {
            Ok(rustix::fs::renameat(
                &from_dir.0,
                from.as_os_str(),
                &to_dir.0,
                to.as_os_str(),
            )?)
        })
    }

    fn rename_noreplace(
        &self,
        from_dir: &RealDir,
        from: &Name,
        to_dir: &RealDir,
        to: &Name,
    ) -> io::Result<()> {
        operation(Boundary::TreePublish, label(to), || {
            Ok(rustix::fs::renameat_with(
                &from_dir.0,
                from.as_os_str(),
                &to_dir.0,
                to.as_os_str(),
                RenameFlags::NOREPLACE,
            )?)
        })
    }

    fn unlink(&self, parent: &RealDir, name: &Name) -> io::Result<()> {
        operation(Boundary::Unlink, label(name), || {
            Ok(rustix::fs::unlinkat(
                &parent.0,
                name.as_os_str(),
                AtFlags::empty(),
            )?)
        })
    }

    fn symlink(&self, parent: &RealDir, name: &Name, target: &Path) -> io::Result<()> {
        operation(Boundary::Symlink, label(name), || {
            Ok(rustix::fs::symlinkat(target, &parent.0, name.as_os_str())?)
        })
    }

    fn read_link(&self, parent: &RealDir, name: &Name) -> io::Result<PathBuf> {
        operation(Boundary::Read, label(name), || {
            let target = rustix::fs::readlinkat(&parent.0, name.as_os_str(), Vec::new())?;
            Ok(PathBuf::from(bytes_to_os(target.into_bytes())))
        })
    }

    fn metadata(&self, parent: &RealDir, name: &Name) -> io::Result<Metadata> {
        operation(Boundary::Read, label(name), || {
            let stat = rustix::fs::statat(&parent.0, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)?;
            let kind = match FileType::from_raw_mode(stat.st_mode) {
                FileType::RegularFile => ObjectKind::File,
                FileType::Directory => ObjectKind::Dir,
                FileType::Symlink => ObjectKind::Symlink,
                _ => ObjectKind::Other,
            };
            #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
            Ok(Metadata {
                kind,
                len: stat.st_size as u64,
                permissions: Permissions::new(u32::from(stat.st_mode)),
                object: ObjectId {
                    filesystem: stat.st_dev as u64,
                    inode: stat.st_ino as u64,
                },
            })
        })
    }

    fn read_dir(&self, dir: &RealDir) -> io::Result<Vec<Name>> {
        operation(Boundary::Read, Path::new("<dir>"), || {
            let mut names = Vec::new();
            for entry in rustix::fs::Dir::read_from(&dir.0)? {
                let entry = entry?;
                let bytes = entry.file_name().to_bytes();
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                names.push(Name::new(bytes_to_os(bytes.to_vec()))?);
            }
            names.sort();
            Ok(names)
        })
    }
}

#[cfg(unix)]
fn bytes_to_os(bytes: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use rustix::fs::FileType;

    #[test]
    fn open_file_refuses_a_fifo_without_blocking() {
        let home = tempfile::tempdir().unwrap();
        let fs = RealFs::new(home.path());
        let root = fs.root().unwrap();
        let fifo = Name::new("pipe").unwrap();
        rustix::fs::mknodat(
            &root.0,
            fifo.as_os_str(),
            FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        assert_eq!(fs.metadata(&root, &fifo).unwrap().kind, ObjectKind::Other);
        assert_eq!(
            fs.open_file(&root, &fifo).unwrap_err().raw_os_error(),
            Some(Errno::INVAL.raw_os_error())
        );
    }
}
