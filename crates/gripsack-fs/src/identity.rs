//! Filesystem identity plus immutable inode birth identity, never st_dev or a
//! mount ID. Ext4 derives fsid from its on-disk UUID; other filesystem lifetime
//! guarantees require qualification. Missing birth/FS identity fails closed.
use crate::Dir;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryObjectIdentity {
    pub filesystem_kind: i64,
    pub filesystem_id: u64,
    pub inode: u64,
    pub birth_seconds: i64,
    pub birth_nanoseconds: u32,
}

pub fn directory_object_identity(directory: &Dir) -> io::Result<DirectoryObjectIdentity> {
    use rustix::fs::{AtFlags, StatxFlags};
    let requested = StatxFlags::TYPE | StatxFlags::INO | StatxFlags::BTIME;
    let object = rustix::fs::statx(directory, "", AtFlags::EMPTY_PATH, requested)?;
    if object.stx_mask & requested.bits() != requested.bits()
        || u32::from(object.stx_mode) & libc::S_IFMT != libc::S_IFDIR
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory transitions require filesystem inode birth identity; no st_dev-only fallback is permitted",
        ));
    }
    let filesystem = rustix::fs::fstatfs(directory)?;
    let portable = rustix::fs::fstatvfs(directory)?;
    if portable.f_fsid == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem provides no persistent identity for directory ownership",
        ));
    }
    Ok(DirectoryObjectIdentity {
        filesystem_kind: filesystem.f_type,
        filesystem_id: portable.f_fsid,
        inode: object.stx_ino,
        birth_seconds: object.stx_btime.tv_sec,
        birth_nanoseconds: object.stx_btime.tv_nsec,
    })
}
