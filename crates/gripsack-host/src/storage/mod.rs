//! The storage boundary of ring 2 (plan/0056 §4).
//!
//! Every operation names one directory entry relative to a pinned directory
//! handle: there are no multi-component paths, so a walk is a sequence of
//! [`Storage::open_dir`] calls, each refusing to follow a symlink. That is
//! the O_NOFOLLOW parent pinning the driver needs to compute `Live` and then
//! act on the same parent inode.
//!
//! The trait covers exactly what the storage model talks about:
//!
//! - S1: [`Storage::rename`] within a directory is atomic and replaces the
//!   target; [`Storage::rename_noreplace`] and [`Storage::create_dir`] fail
//!   with `AlreadyExists` if the name exists; [`Storage::remove_tree`] is
//!   *not* atomic, it is the sequence of unlinks it issues.
//! - S2: [`Storage::sync_file`] makes a file's bytes and mode durable;
//!   [`Storage::sync_dir`] makes the directory's names durable.
//! - S3: crash kinds are a property of an implementation's environment, not
//!   of this interface; [`MemFs`] injects both.
//!
//! Errors are `io::Error`s carrying the platform errno, so callers match on
//! `kind()` (or `raw_os_error()`) the same way for both implementations.

mod mem;
mod real;

pub use gripsack_fs::fault::Boundary;
pub use mem::{
    Crash, CrashKinds, CrashPoint, Exploration, ExploreError, ExternalWrite, MemDir, MemFile,
    MemFs, Operation, PendingBound, TraceEvent, Violation, explore,
};
pub use real::{RealDir, RealFile, RealFs};

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// One directory entry name: a single, non-empty path component that is
/// neither `.` nor `..` and contains no `/` or NUL. Construction is the
/// only check; a `Name` can never address another directory.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(OsString);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidName(pub OsString);

impl fmt::Display for InvalidName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not a single directory entry name", self.0)
    }
}

impl std::error::Error for InvalidName {}

impl From<InvalidName> for io::Error {
    fn from(error: InvalidName) -> Self {
        io::Error::new(io::ErrorKind::InvalidInput, error)
    }
}

impl Name {
    pub fn new(name: impl Into<OsString>) -> Result<Self, InvalidName> {
        let name = name.into();
        let bytes = name.as_encoded_bytes();
        let valid = !bytes.is_empty()
            && bytes != b"."
            && bytes != b".."
            && !bytes.iter().any(|&b| b == b'/' || b == 0);
        if valid {
            Ok(Self(name))
        } else {
            Err(InvalidName(name))
        }
    }

    pub fn as_os_str(&self) -> &OsStr {
        &self.0
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// Permission bits (`0o7777` at most); the file type lives in [`ObjectKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions(u32);

impl Permissions {
    pub const fn new(bits: u32) -> Self {
        Self(bits & 0o7777)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Permissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04o}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    File,
    Dir,
    Symlink,
    /// Sockets, devices, FIFOs: never produced by gripsack, only observed.
    Other,
}

/// Which object a name currently resolves to: filesystem plus inode. Equal
/// identities mean the same inode, not the same bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    pub filesystem: u64,
    pub inode: u64,
}

/// `lstat` of one entry: never follows a final symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    pub kind: ObjectKind,
    pub len: u64,
    pub permissions: Permissions,
    pub object: ObjectId,
}

/// The filesystem operations of the storage model, inode-based: handles pin
/// inodes, names are single entries relative to a pinned directory.
///
/// Implementations take `&self`: handles are the per-operation authority,
/// the storage value is shared.
pub trait Storage {
    /// A pinned, syncable directory.
    type Dir: fmt::Debug;
    /// An open regular file. Handles from [`Storage::create_file`] write;
    /// handles from [`Storage::open_file`] only read and sync.
    type File: fmt::Debug;

    /// The ambient root this storage was opened on.
    fn root(&self) -> io::Result<Self::Dir>;

    /// Open a child directory without following a symlink (`O_NOFOLLOW`):
    /// a symlink or non-directory at `name` is an error.
    fn open_dir(&self, parent: &Self::Dir, name: &Name) -> io::Result<Self::Dir>;

    /// Exclusive mkdir (S1): `AlreadyExists` if anything is at `name`.
    fn create_dir(&self, parent: &Self::Dir, name: &Name, mode: Permissions) -> io::Result<()>;

    /// rmdir: the directory must be empty.
    fn remove_dir(&self, parent: &Self::Dir, name: &Name) -> io::Result<()>;

    /// Exclusive create (`O_CREAT|O_EXCL|O_NOFOLLOW`), opened for writing at
    /// offset zero. `mode` is subject to the process umask on a real
    /// filesystem; callers that need exact bits call [`Storage::set_mode`].
    fn create_file(
        &self,
        parent: &Self::Dir,
        name: &Name,
        mode: Permissions,
    ) -> io::Result<Self::File>;

    /// Open an existing regular file without following a symlink, for
    /// reading and syncing. A directory or other kind is an error.
    fn open_file(&self, parent: &Self::Dir, name: &Name) -> io::Result<Self::File>;

    /// Append all of `bytes` at the handle's position.
    fn write(&self, file: &mut Self::File, bytes: &[u8]) -> io::Result<()>;

    /// `fchmod` on the open file.
    fn set_mode(&self, file: &Self::File, mode: Permissions) -> io::Result<()>;

    /// The file's whole content, from offset zero.
    fn read_file(&self, file: &Self::File) -> io::Result<Vec<u8>>;

    /// S2 file barrier: the file's bytes and mode are durable.
    fn sync_file(&self, file: &Self::File) -> io::Result<()>;

    /// S2 namespace barrier: this directory's entries are durable.
    fn sync_dir(&self, dir: &Self::Dir) -> io::Result<()>;

    /// Atomic replacing rename (S1). Across two directories each side's
    /// entry needs its own directory barrier.
    fn rename(
        &self,
        from_dir: &Self::Dir,
        from: &Name,
        to_dir: &Self::Dir,
        to: &Name,
    ) -> io::Result<()>;

    /// `RENAME_NOREPLACE` (S1): `AlreadyExists` if `to` exists.
    fn rename_noreplace(
        &self,
        from_dir: &Self::Dir,
        from: &Name,
        to_dir: &Self::Dir,
        to: &Name,
    ) -> io::Result<()>;

    /// Remove a non-directory entry (file or symlink, never followed).
    fn unlink(&self, parent: &Self::Dir, name: &Name) -> io::Result<()>;

    /// Create a symlink holding `target` verbatim; nothing is resolved.
    fn symlink(&self, parent: &Self::Dir, name: &Name, target: &Path) -> io::Result<()>;

    fn read_link(&self, parent: &Self::Dir, name: &Name) -> io::Result<PathBuf>;

    /// `lstat` of the entry.
    fn metadata(&self, parent: &Self::Dir, name: &Name) -> io::Result<Metadata>;

    /// The directory's entry names, sorted, without `.` and `..`.
    fn read_dir(&self, dir: &Self::Dir) -> io::Result<Vec<Name>>;

    /// Open, read whole, close.
    fn read(&self, parent: &Self::Dir, name: &Name) -> io::Result<Vec<u8>> {
        let file = self.open_file(parent, name)?;
        self.read_file(&file)
    }

    /// Remove `name` and everything below it, children first. Not atomic
    /// (S1): a crash leaves any prefix of the unlinks, and no barrier is
    /// issued, so a power loss may keep any subset of them.
    fn remove_tree(&self, parent: &Self::Dir, name: &Name) -> io::Result<()> {
        if self.metadata(parent, name)?.kind != ObjectKind::Dir {
            return self.unlink(parent, name);
        }
        let dir = self.open_dir(parent, name)?;
        for child in self.read_dir(&dir)? {
            self.remove_tree(&dir, &child)?;
        }
        self.remove_dir(parent, name)
    }
}

#[cfg(test)]
mod conformance;
