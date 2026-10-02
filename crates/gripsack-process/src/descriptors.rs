//! Apply close-on-exec in the child, retaining only an explicitly supplied
//! script handle. Marking rather than closing preserves Rust's private exec
//! error pipe until its normal close-on-exec handshake.
#[cfg(target_os = "linux")]
mod linux;
use std::{io, os::fd::RawFd};

pub(crate) struct DescriptorPolicy {
    #[cfg(target_os = "macos")]
    maximum: libc::c_int,
}

impl DescriptorPolicy {
    pub(crate) fn admit() -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {})
        }
        #[cfg(target_os = "macos")]
        {
            let mut maximum: libc::c_int = 0;
            let mut length = std::mem::size_of_val(&maximum);
            // SAFETY: the fixed sysctl name and writable output/length pointers
            // have the declared sizes; this runs before fork, not in pre_exec.
            let result = unsafe {
                libc::sysctlbyname(
                    c"kern.maxfilesperproc".as_ptr(),
                    (&mut maximum as *mut libc::c_int).cast(),
                    &mut length,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if result < 0 {
                return Err(io::Error::last_os_error());
            }
            if length != std::mem::size_of_val(&maximum) || maximum < 3 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid kernel descriptor ceiling",
                ));
            }
            Ok(Self { maximum })
        }
    }

    /// Only async-signal-safe descriptor syscalls execute after fork. Kernel
    /// descriptor limits must not be lowered by a privileged concurrent actor.
    pub(crate) fn apply(&self, script: Option<RawFd>) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            // SAFETY: close_range with CLOEXEC changes only descriptor flags in
            // this child. Older kernels use child-local enumeration below;
            // policy denial still refuses launch instead of weakening closure.
            let result = unsafe {
                libc::syscall(
                    libc::SYS_close_range,
                    3_u32,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                )
            };
            if result < 0 {
                let error = io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(libc::ENOSYS | libc::EINVAL)) {
                    linux::mark_all_cloexec()?;
                } else {
                    return Err(error);
                }
            }
        }
        #[cfg(target_os = "macos")]
        for descriptor in 3..self.maximum {
            // SAFETY: integer descriptor queries/flag updates do not dereference
            // pointers. No other thread opens descriptors in the forked child.
            let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
            if flags < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EBADF) {
                    return Err(error);
                }
            } else if unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC) }
                < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        if let Some(descriptor) = script {
            // SAFETY: this is the retained sealed script fd, intentionally made
            // available to the separately admitted interpreter after exec.
            if unsafe { libc::fcntl(descriptor, libc::F_SETFD, 0) } < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

/// Retained authority must not occupy fd 0/1/2: Command's child-side stdio
/// setup replaces those slots before our pre_exec hook runs.
pub(crate) fn retain_above_stdio(file: std::fs::File) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    if file.as_raw_fd() >= 3 {
        return Ok(file);
    }
    // SAFETY: fcntl returns a new owned descriptor at or above the first
    // non-stdio slot. The old File is dropped only after duplication succeeds.
    let descriptor = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(descriptor) })
}
