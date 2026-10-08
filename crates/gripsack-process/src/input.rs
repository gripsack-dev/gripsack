//! Bounded serializer bytes and allocation requests; allocator granularity is external.

use super::InputByteLimit;
use gripsack_policy::process_budget::admit_input_append;
use std::io::{self, Write};

pub struct InputBuffer {
    bytes: Vec<u8>,
    limit: InputByteLimit,
}

impl InputBuffer {
    pub fn new(limit: InputByteLimit) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Transfer the already bounded request without copying it into a second
    /// allocation. Framed callers can fill their reserved header afterward.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Read a Linux inherited auxiliary input without borrowing stdin.
    /// Pipes use the same nonblocking/deadline poll boundary as process exchange;
    /// unsupported special files are never accepted as bounded context input.
    /// This dedicated one-shot input loses inheritance authority: fd itself is
    /// marked close-on-exec, including on later admission failure. Reading uses
    /// an independent description and never changes shared status flags or
    /// consumes an alias of standard IO.
    pub fn read_inherited(
        fd: std::os::fd::RawFd,
        limit: InputByteLimit,
        end: std::time::Instant,
    ) -> io::Result<Self> {
        use std::io::Read;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
        if fd <= libc::STDERR_FILENO {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "auxiliary input must not borrow standard IO",
            ));
        }
        // SAFETY: fcntl validates the supplied integer; successful duplication
        // gives this function one owned descriptor, never ownership of fd.
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: duplicate is a new, live descriptor exclusively owned here.
        let source = unsafe { std::fs::File::from_raw_fd(duplicate) };
        // Mark the source close-on-exec too: no later child may inherit context.
        // SAFETY: fd remains borrowed from the caller, and these take no pointers.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        // SAFETY: setting descriptor flags preserves the live borrowed handle.
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let metadata = source.metadata()?;
        if !metadata.is_file() && !metadata.file_type().is_fifo() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "auxiliary input must be a regular file or pipe",
            ));
        }
        for standard in libc::STDIN_FILENO..=libc::STDERR_FILENO {
            let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: fstat initializes the complete writable stat on success;
            // an absent standard descriptor contributes no input authority.
            if unsafe { libc::fstat(standard, status.as_mut_ptr()) } == 0 {
                // SAFETY: the successful fstat above initialized status.
                let status = unsafe { status.assume_init() };
                if status.st_dev as u128 == u128::from(metadata.dev())
                    && status.st_ino as u128 == u128::from(metadata.ino())
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "auxiliary input aliases standard IO",
                    ));
                }
            }
        }
        // Reopen the pinned Linux descriptor with an independent open-file
        // description: setting O_NONBLOCK on dup alone would mutate the
        // caller's shared flags (possibly shared with another process).
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(format!("/proc/self/fd/{}", source.as_raw_fd()))?;
        let reopened = file.metadata()?;
        if reopened.dev() != metadata.dev()
            || reopened.ino() != metadata.ino()
            || reopened.file_type() != metadata.file_type()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "auxiliary input identity changed while reopening",
            ));
        }
        drop(source);
        let mut deadline = super::OperationDeadline::at(end);
        let mut input = Self::new(limit);
        let mut buffer = [0u8; 4096];
        loop {
            let remaining = deadline.remaining().ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "auxiliary input deadline expired")
            })?;
            let request = buffer.len().min(
                limit
                    .bytes()
                    .saturating_sub(input.bytes.len())
                    .saturating_add(1),
            );
            match file.read(&mut buffer[..request]) {
                Ok(0) => {
                    if deadline.remaining().is_none() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "auxiliary input deadline expired",
                        ));
                    }
                    return Ok(input);
                }
                Ok(count) => {
                    input.write_all(&buffer[..count])?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let mut fds = [super::sys::pollfd(Some(file.as_raw_fd()), libc::POLLIN)];
                    super::sys::poll(&mut fds, remaining)?;
                    if fds[0].revents & libc::POLLNVAL != 0 {
                        return Err(io::Error::from_raw_os_error(libc::EBADF));
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Write for InputBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(append) = admit_input_append(
            self.limit,
            self.bytes.len(),
            self.bytes.capacity(),
            bytes.len(),
        ) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("request exceeds the {} byte cap", self.limit.bytes()),
            ));
        };
        if append.reserve_additional() != 0 {
            self.bytes.reserve_exact(append.reserve_additional());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_append_preserves_the_admitted_request() {
        let mut input = InputBuffer::new(InputByteLimit::new(17));
        input.write_all(b"abcd").unwrap();
        input.write_all(b"efghijklmnop").unwrap();
        let error = input
            .write_all(b"QR")
            .expect_err("serializer_excess_was_admitted");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            input.as_bytes(),
            b"abcdefghijklmnop",
            "rejected_serializer_append_changed_request",
        );
        input.write_all(b"q").unwrap();
        assert_eq!(input.as_bytes(), b"abcdefghijklmnopq");
    }

    #[cfg(target_os = "linux")]
    fn pipe() -> (std::fs::File, std::fs::File) {
        use std::os::fd::FromRawFd;
        let mut fds = [-1; 2];
        // SAFETY: the array supplies exactly two writable descriptor slots.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: pipe returned two distinct live descriptors, each owned here.
        unsafe {
            (
                std::fs::File::from_raw_fd(fds[0]),
                std::fs::File::from_raw_fd(fds[1]),
            )
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_auxiliary_input_enforces_deadline_and_byte_cap() {
        use std::os::fd::AsRawFd;
        use std::time::{Duration, Instant};
        let (reader, mut writer) = pipe();
        let alias = reader.try_clone().unwrap();
        let timeout = InputBuffer::read_inherited(
            reader.as_raw_fd(),
            InputByteLimit::new(4),
            Instant::now() + Duration::from_millis(20),
        )
        .err()
        .expect("held-open empty pipe must time out");
        assert_eq!(timeout.kind(), io::ErrorKind::TimedOut);
        assert_blocking(&reader);
        assert_blocking(&alias);
        writer.write_all(b"five!").unwrap();
        drop(writer);
        let overflow = InputBuffer::read_inherited(
            reader.as_raw_fd(),
            InputByteLimit::new(4),
            Instant::now() + Duration::from_secs(1),
        )
        .err()
        .expect("oversized context must be refused");
        assert_eq!(overflow.kind(), io::ErrorKind::InvalidInput);
        assert_blocking(&reader);
        assert_blocking(&alias);
        let (reader, mut writer) = pipe();
        let alias = reader.try_clone().unwrap();
        writer.write_all(b"ok\n").unwrap();
        drop(writer);
        let input = InputBuffer::read_inherited(
            reader.as_raw_fd(),
            InputByteLimit::new(4),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(input.as_bytes(), b"ok\n");
        assert_blocking(&reader);
        assert_blocking(&alias);
        // SAFETY: reader retains its valid descriptor; F_GETFD takes no pointer.
        assert_ne!(
            unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_auxiliary_input_refuses_standard_and_special_descriptors() {
        use std::os::fd::AsRawFd;
        use std::time::{Duration, Instant};
        let end = Instant::now() + Duration::from_secs(1);
        let error = InputBuffer::read_inherited(0, InputByteLimit::new(4), end)
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let error = InputBuffer::read_inherited(socket.as_raw_fd(), InputByteLimit::new(4), end)
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(target_os = "linux")]
    fn assert_blocking(file: &std::fs::File) {
        use std::os::fd::AsRawFd;
        // SAFETY: the file owns a live descriptor; F_GETFL takes no pointer.
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(flags & libc::O_NONBLOCK, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_auxiliary_input_refuses_stdin_alias_without_draining_it() {
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        const CHILD: &str = "GRIPSACK_INPUT_TEST_STDIN_ALIAS";
        if std::env::var_os(CHILD).is_some() {
            let error = InputBuffer::read_inherited(
                3,
                InputByteLimit::new(64),
                Instant::now() + Duration::from_secs(1),
            )
            .err()
            .expect("stdin alias must be refused");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            assert_eq!(input, "untouched standard input\n");
            return;
        }
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exec 3<&0; exec \"$1\" --exact input::tests::inherited_auxiliary_input_refuses_stdin_alias_without_draining_it --nocapture", "auxiliary-input-test"])
            .arg(std::env::current_exe().unwrap()).env(CHILD, "1")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"untouched standard input\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "the stdin-alias child regression was not selected"
        );
    }
}
