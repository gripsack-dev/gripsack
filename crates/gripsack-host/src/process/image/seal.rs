use rustix::fs::{SealFlags, fcntl_add_seals};
use std::{
    fs::File,
    io,
    time::{Duration, Instant},
};

const REQUIRED: SealFlags = SealFlags::WRITE
    .union(SealFlags::GROW)
    .union(SealFlags::SHRINK)
    .union(SealFlags::SEAL);

pub(super) fn executable(
    file: &File,
    deadline: Instant,
    mut wait: impl FnMut(Duration),
) -> io::Result<()> {
    // Linux can report EBUSY even without writable mappings: a temporary
    // reference to a shmem page can outlive the kernel's F_SEAL_WRITE scan.
    let mut retries = 3;
    while Instant::now() < deadline {
        let result = fcntl_add_seals(file, REQUIRED);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if result != Err(rustix::io::Errno::BUSY) || retries == 0 {
            return result.map_err(Into::into);
        }
        retries -= 1;
        wait(remaining.min(Duration::from_millis(10)));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "native image sealing deadline expired",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::fs::{MemfdFlags, fcntl_get_seals, memfd_create};
    use std::{io::Write, os::fd::AsRawFd};

    fn pinned_image() -> (File, io::PipeReader) {
        let mut file = File::from(memfd_create(c"test-image", MemfdFlags::ALLOW_SEALING).unwrap());
        file.write_all(&[42; 4096]).unwrap();
        let (reader, writer) = io::pipe().unwrap();
        let mut offset = 0;
        // SAFETY: both descriptors are live, offset is writable and the pipe
        // end has no offset. One page remains referenced until the reader drops.
        let n = unsafe {
            libc::splice(
                file.as_raw_fd(),
                &mut offset,
                writer.as_raw_fd(),
                std::ptr::null_mut(),
                4096,
                0,
            )
        };
        assert_eq!(n, 4096);
        (file, reader)
    }

    #[test]
    fn released_read_pin_allows_only_the_fully_sealed_image() {
        let (mut file, reader) = pinned_image();
        let mut pin = Some(reader);
        executable(&file, Instant::now() + Duration::from_secs(10), |_| {
            drop(pin.take());
        })
        .unwrap();
        assert!(pin.is_none());
        assert!(fcntl_get_seals(&file).unwrap().contains(REQUIRED));
        assert_eq!(
            file.write(b"!").unwrap_err().raw_os_error(),
            Some(libc::EPERM)
        );
    }

    #[test]
    fn persistent_read_pin_refuses_without_partial_seals() {
        let (file, _pin) = pinned_image();
        let before = fcntl_get_seals(&file).unwrap();
        let err = executable(&file, Instant::now() + Duration::from_secs(10), |_| {}).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EBUSY));
        assert_eq!(fcntl_get_seals(&file).unwrap(), before);
    }

    #[test]
    fn expired_selection_cannot_admit_even_a_sealable_image() {
        let (file, pin) = pinned_image();
        drop(pin);
        let before = fcntl_get_seals(&file).unwrap();
        let err = executable(&file, Instant::now(), |_| {}).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert_eq!(fcntl_get_seals(&file).unwrap(), before);
    }
}
