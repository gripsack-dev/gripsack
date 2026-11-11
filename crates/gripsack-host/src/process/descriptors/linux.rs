//! Allocation-free pre_exec fallback when close_range(CLOEXEC) is unavailable.
//! Enumerate the CHILD's live descriptor table, not a racy pre-fork parent
//! snapshot or a potentially lowered RLIMIT_NOFILE. The getdents64 ABI is from
//! Linux getdents(2); no directory-library allocator runs after fork.
use std::io;

#[repr(C)]
struct LinuxDirent64 {
    _inode: u64,
    _offset: i64,
    record_bytes: u16,
    _entry_type: u8,
    name: [u8; 0],
}

struct Directory(libc::c_int);
impl Drop for Directory {
    fn drop(&mut self) {
        // SAFETY: this child owns the descriptor returned by open below.
        unsafe {
            libc::close(self.0);
        }
    }
}

pub(super) fn mark_all_cloexec() -> io::Result<()> {
    // SAFETY: fixed terminated path; open is async-signal-safe. Opening here,
    // after fork, binds /proc/self to the child rather than the live parent.
    let fd = unsafe {
        libc::open(
            c"/proc/self/fd".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let directory = Directory(fd);
    let mut buffer = [0_u8; 8192];
    loop {
        // SAFETY: a live directory descriptor and a valid fixed writable buffer;
        // getdents64 copies at most the supplied buffer size.
        let count = unsafe {
            libc::syscall(
                libc::SYS_getdents64,
                directory.0,
                buffer.as_mut_ptr(),
                buffer.len(),
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if count == 0 {
            return Ok(());
        }
        let count = count as usize;
        if count > buffer.len() {
            return Err(invalid());
        }
        let mut position = 0;
        while position < count {
            let remaining = &buffer[position..count];
            let name_offset = std::mem::offset_of!(LinuxDirent64, name);
            let size_offset = std::mem::offset_of!(LinuxDirent64, record_bytes);
            if remaining.len() <= name_offset {
                return Err(invalid());
            }
            let length =
                u16::from_ne_bytes([remaining[size_offset], remaining[size_offset + 1]]) as usize;
            if length <= name_offset || length > remaining.len() {
                return Err(invalid());
            }
            let name = &remaining[name_offset..length];
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .ok_or_else(invalid)?;
            let name = &name[..end];
            if name != b"." && name != b".." {
                if name.is_empty() {
                    return Err(invalid());
                }
                let mut descriptor = 0_i32;
                for &byte in name {
                    if !byte.is_ascii_digit() {
                        return Err(invalid());
                    }
                    descriptor = descriptor
                        .checked_mul(10)
                        .and_then(|value| value.checked_add(i32::from(byte - b'0')))
                        .ok_or_else(invalid)?;
                }
                if descriptor >= 3 {
                    // SAFETY: only descriptor flags are changed; enumeration is
                    // stable because the forked child has no competing thread.
                    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
                    if flags < 0 {
                        let error = io::Error::last_os_error();
                        if error.raw_os_error() != Some(libc::EBADF) {
                            return Err(error);
                        }
                    } else if unsafe {
                        libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC)
                    } < 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
            }
            position += length;
        }
    }
}

fn invalid() -> io::Error {
    io::Error::from_raw_os_error(libc::EIO)
}
