//! Private executable materialization. Linux seals a memfd and executes its
//! retained descriptor; macOS uses a private read-only copy and reports the
//! weaker pathname tier explicitly. Neither binds dynamic libraries or native
//! subprocesses selected by the launched program.
use super::{ByteBinding, OperatorEnvironment, Sha256Digest};
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    os::unix::{
        ffi::OsStringExt,
        fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

const EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
const HEADER_BYTES: usize = 4096;

pub(crate) struct Image {
    pub(crate) digest: Sha256Digest,
    pub(crate) binding: ByteBinding,
    pub(crate) path: PathBuf,
    pub(crate) file: File,
    #[cfg(target_os = "macos")]
    _directory: tempfile::TempDir,
}

impl Image {
    fn copy(
        source: &Path,
        expected: Option<Sha256Digest>,
        deadline: std::time::Instant,
    ) -> io::Result<Self> {
        let mut input = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(source)?;
        let metadata = input.metadata()?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "selected native executable is not an executable regular file",
            ));
        }
        if metadata.len() > EXECUTABLE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "native executable exceeds its admission budget",
            ));
        }
        #[cfg(target_os = "linux")]
        let (mut output, path, binding) = {
            // SAFETY: the name is a static terminated C string; a successful
            // return is a new owned descriptor, transferred immediately.
            let flags = libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING;
            let descriptor = unsafe {
                libc::memfd_create(c"gripsack-native-image".as_ptr(), flags | libc::MFD_EXEC)
            };
            let fd = if descriptor < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EINVAL) {
                    return Err(error);
                }
                // Pre-MFD_EXEC kernels reject the new bit. The legacy form
                // remains sealed/descriptor-bound, not a pathname fallback.
                unsafe { libc::memfd_create(c"gripsack-native-image".as_ptr(), flags) }
            } else {
                descriptor
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = super::descriptors::retain_above_stdio(unsafe { File::from_raw_fd(fd) })?;
            let fd = file.as_raw_fd();
            (
                file,
                PathBuf::from(format!("/proc/self/fd/{fd}")),
                ByteBinding::ExecutableHandle,
            )
        };
        #[cfg(target_os = "macos")]
        let directory = tempfile::Builder::new()
            .prefix("gripsack-native-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()?;
        #[cfg(target_os = "macos")]
        let (mut output, path, binding) = {
            let path = directory.path().join("image");
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o500)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&path)?;
            (file, path, ByteBinding::PrivateCopyPath)
        };
        let mut hasher = Sha256::new();
        let mut buffer = [0; 32 * 1024];
        let mut total = 0_u64;
        loop {
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "native image admission deadline expired",
                ));
            }
            let length = input.read(&mut buffer)?;
            if length == 0 {
                break;
            }
            total = total.checked_add(length as u64).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "native executable size overflow",
                )
            })?;
            if total > EXECUTABLE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "native executable exceeds its admission budget",
                ));
            }
            let bytes = &buffer[..length];
            hasher.update(bytes);
            output.write_all(bytes)?;
        }
        let digest = Sha256Digest::from_bytes(hasher.finalize().into());
        if expected.is_some_and(|approved| approved != digest) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "selected executable bytes no longer match their approved digest",
            ));
        }
        output.set_permissions(std::fs::Permissions::from_mode(0o500))?;
        #[cfg(target_os = "linux")]
        {
            // SAFETY: this owned memfd has no writable mappings. Refuse a
            // platform that cannot seal it; no hash/reopen fallback is hidden.
            let status = unsafe {
                libc::fcntl(
                    output.as_raw_fd(),
                    libc::F_ADD_SEALS,
                    libc::F_SEAL_WRITE
                        | libc::F_SEAL_GROW
                        | libc::F_SEAL_SHRINK
                        | libc::F_SEAL_SEAL,
                )
            };
            if status < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(target_os = "macos")]
        let output = {
            output.sync_all()?;
            drop(output); // an open writer would cause ETXTBSY at exec
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)?
        };
        #[cfg(target_os = "macos")]
        let output = super::descriptors::retain_above_stdio(output)?;
        Ok(Self {
            digest,
            binding,
            path,
            file: output,
            #[cfg(target_os = "macos")]
            _directory: directory,
        })
    }

    fn interpreter(&self) -> io::Result<Option<(PathBuf, Option<OsString>)>> {
        use std::os::unix::fs::FileExt;
        let mut magic = [0; 2];
        self.file.read_exact_at(&mut magic, 0).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "selected native executable has no complete image header",
                )
            } else {
                error
            }
        })?;
        if magic != *b"#!" {
            return Ok(None);
        }
        let length = self.file.metadata()?.len().min(HEADER_BYTES as u64) as usize;
        let mut header = [0; HEADER_BYTES];
        self.file.read_exact_at(&mut header[..length], 0)?;
        let end = header[..length]
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "native script has an oversized or unterminated interpreter line",
                )
            })?;
        let line = header[2..end].trim_ascii();
        let split = line
            .iter()
            .position(|byte| matches!(byte, b' ' | b'\t'))
            .unwrap_or(line.len());
        let interpreter = PathBuf::from(OsString::from_vec(line[..split].to_vec()));
        if !interpreter.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "native script interpreter must be absolute",
            ));
        }
        let rest = line[split..].trim_ascii();
        let argument = (!rest.is_empty()).then(|| OsString::from_vec(rest.to_vec()));
        Ok(Some((interpreter, argument)))
    }
}

pub(crate) struct Program {
    pub(crate) executable: Image,
    pub(crate) script: Option<Image>,
    pub(crate) interpreter_argument: Option<OsString>,
    pub(crate) argument_zero: OsString,
}

impl Program {
    pub(crate) fn select(
        environment: &OperatorEnvironment,
        name: &Path,
        expected: Option<Sha256Digest>,
        deadline: std::time::Instant,
    ) -> io::Result<Self> {
        let selected = Image::copy(&environment.resolve(name)?, expected, deadline)?;
        let Some((interpreter, argument)) = selected.interpreter()? else {
            return Ok(Self {
                executable: selected,
                script: None,
                interpreter_argument: None,
                argument_zero: name.as_os_str().to_owned(),
            });
        };
        let executable = Image::copy(&environment.resolve(&interpreter)?, None, deadline)?;
        if executable.interpreter()?.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "recursive native script interpreters are not an admitted execution form",
            ));
        }
        Ok(Self {
            executable,
            script: Some(selected),
            interpreter_argument: argument,
            argument_zero: interpreter.into_os_string(),
        })
    }
}
